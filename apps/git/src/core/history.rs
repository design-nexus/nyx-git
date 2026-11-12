//! A cancellable history actor owns gitoxide, traversal and disk-backed layout.
use crate::graph::{
    layout::Layout,
    model::{Commit, Decoration, Query, RefKind, Row},
    virtualization::{MAX_CACHE, PREFETCH, Store},
};
use anyhow::{Result, ensure};
use gix::objs::Find;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    Head,
    Reference(String),
}
#[derive(Debug)]
pub enum Request {
    Page { start: usize, count: usize },
    Find { query: Query, from: usize, backwards: bool },
    Locate { id: String, within: usize },
    End,
}
#[derive(Debug)]
pub enum Event {
    Ready { refs: Vec<Decoration> },
    Page { start: usize, rows: Vec<Row>, scanned: usize, done: bool, lanes: usize },
    Found { index: Option<usize>, scanned: usize, done: bool, lanes: usize },
    Error(String),
}
pub struct Session {
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
}
impl Session {
    pub fn start(root: PathBuf, scope: Scope) -> Self {
        Self::start_with(root, scope, false)
    }
    /// With `first_parent`, merges show only their first parent: merged side branches
    /// collapse into the merge commit, which records how many parents it hides.
    pub fn start_with(root: PathBuf, scope: Scope, first_parent: bool) -> Self {
        let (tx, requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        thread::spawn(move || {
            if let Err(e) = serve(&root, scope, first_parent, requests, &events, stop.clone())
                && !stop.load(Ordering::Relaxed)
            {
                let _ = events.send(Event::Error(format!("{e:#}")));
            }
        });
        Self { tx, rx, cancel }
    }
    pub fn request(&self, request: Request) -> bool {
        self.tx.send(request).is_ok()
    }
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The topo walker has no shallow-boundary adapter. Present shallow commits as
/// roots to traversal, without changing the original objects or repository.
struct Finder<'a> {
    repo: &'a gix::Repository,
    shallow: HashSet<gix::ObjectId>,
    cancel: Arc<AtomicBool>,
}
impl Find for Finder<'_> {
    fn try_find<'a>(
        &self,
        id: &gix::hash::oid,
        buffer: &'a mut Vec<u8>,
    ) -> std::result::Result<Option<gix::objs::Data<'a>>, gix::objs::find::Error> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "History cancelled").into());
        }
        if !self.shallow.contains(id) {
            return self.repo.objects.try_find(id, buffer);
        }
        let Some(object) = self.repo.objects.try_find(id, buffer)? else { return Ok(None) };
        let (kind, object_hash) = (object.kind, object.object_hash);
        let mut bytes = Vec::with_capacity(object.data.len());
        let mut header = true;
        for line in object.data.split_inclusive(|b| *b == b'\n') {
            if line == b"\n" {
                header = false;
            }
            if !(header && line.starts_with(b"parent ")) {
                bytes.extend_from_slice(line);
            }
        }
        *buffer = bytes;
        Ok(Some(gix::objs::Data { kind, object_hash, data: buffer }))
    }
}
/// Each commit's ref labels, and the tips to walk from.
pub type References = (HashMap<gix::ObjectId, Vec<Decoration>>, Vec<gix::ObjectId>);
pub fn references(repo: &gix::Repository) -> Result<References> {
    let mut map: HashMap<_, Vec<_>> = HashMap::new();
    let mut tips = Vec::new();
    if let Ok(id) = repo.head_id() {
        tips.push(id.detach());
        map.entry(id.detach()).or_default().push(Decoration {
            name: "HEAD".into(),
            full_name: "HEAD".into(),
            kind: RefKind::Head,
        });
    }
    let refs = repo.references()?;
    for reference in refs.all()?.peeled()? {
        let reference = reference.map_err(|e| anyhow::anyhow!("Could not read reference: {e}"))?;
        let full = reference.name().as_bstr().to_string();
        let (prefix, kind) = if full.starts_with("refs/heads/") {
            ("refs/heads/", RefKind::Local)
        } else if full.starts_with("refs/remotes/") {
            ("refs/remotes/", RefKind::Remote)
        } else if full.starts_with("refs/tags/") {
            ("refs/tags/", RefKind::Tag)
        } else {
            continue;
        };
        let id = reference.id().detach();
        // Tags can legally refer to blobs or trees; those aren't graph tips.
        if repo.find_object(id)?.kind != gix::objs::Kind::Commit {
            continue;
        }
        tips.push(id);
        map.entry(id).or_default().push(Decoration { name: full[prefix.len()..].into(), full_name: full, kind });
    }
    tips.sort();
    tips.dedup();
    for refs in map.values_mut() {
        refs.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok((map, tips))
}
fn bounded(text: &str, limit: usize) -> String {
    text.chars().take(limit).map(|c| if c.is_control() { ' ' } else { c }).collect()
}
fn commit(
    repo: &gix::Repository,
    id: gix::ObjectId,
    parents: Vec<gix::ObjectId>,
    refs: Vec<Decoration>,
    shallow: bool,
    first_parent: bool,
) -> Result<Commit> {
    let object = repo.find_object(id)?.try_into_commit()?;
    let decoded = object.decode()?;
    let collapsed = if first_parent { parents.len().saturating_sub(1) } else { 0 };
    let parents = if first_parent { parents.into_iter().take(1).collect() } else { parents };
    Ok(Commit {
        id: id.to_string(),
        collapsed,
        parents: parents.into_iter().map(|id| id.to_string()).collect(),
        subject: bounded(decoded.message.to_string().lines().next().unwrap_or_default(), 4096),
        author: bounded(&decoded.author()?.name.to_string(), 512),
        email: bounded(&decoded.author()?.email.to_string(), 512),
        timestamp: decoded.committer()?.time()?.seconds,
        refs,
        shallow,
    })
}
pub struct Cursor<'a> {
    repo: &'a gix::Repository,
    walk: gix::traverse::commit::Topo<Finder<'a>, fn(&gix::hash::oid) -> bool>,
    refs: HashMap<gix::ObjectId, Vec<Decoration>>,
    shallow: HashSet<gix::ObjectId>,
    layout: Layout,
    pub store: Store,
    pub done: bool,
    pub lanes: usize,
    cancel: Arc<AtomicBool>,
    first_parent: bool,
}
impl<'a> Cursor<'a> {
    pub fn new(repo: &'a gix::Repository, scope: Scope, cancel: Arc<AtomicBool>) -> Result<Self> {
        Self::with_options(repo, scope, false, cancel)
    }
    pub fn with_options(repo: &'a gix::Repository, scope: Scope, first_parent: bool, cancel: Arc<AtomicBool>) -> Result<Self> {
        let (refs, all_tips) = references(repo)?;
        let tips = match scope {
            Scope::All => all_tips,
            Scope::Head => repo.head_id().ok().map(|id| id.detach()).into_iter().collect(),
            Scope::Reference(name) => {
                let mut r = repo.find_reference(name.as_str())?;
                vec![r.peel_to_commit()?.id]
            }
        };
        let shallow: HashSet<_> = repo.shallow_commits()?.map(|s| s.iter().copied().collect()).unwrap_or_default();
        let graph = if shallow.is_empty() { repo.commit_graph_if_enabled().ok().flatten() } else { None };
        let find = Finder { repo, shallow: shallow.clone(), cancel: cancel.clone() };
        let walk = gix::traverse::commit::topo::Builder::new(find)
            .with_tips(tips)
            .sorting(gix::traverse::commit::topo::Sorting::TopoOrder)
            .parents(if first_parent { gix::traverse::commit::Parents::First } else { gix::traverse::commit::Parents::All })
            .with_commit_graph(graph)
            .build()?;
        Ok(Self {
            repo,
            walk,
            refs,
            shallow,
            layout: Layout::default(),
            store: Store::new()?,
            done: false,
            lanes: 1,
            cancel,
            first_parent,
        })
    }
    pub fn fill(&mut self, through: usize) -> Result<()> {
        while !self.done && self.store.len() < through {
            ensure!(!self.cancel.load(Ordering::Relaxed), "History cancelled");
            match self.walk.next() {
                None => self.done = true,
                Some(info) => {
                    let info = info?;
                    let commit = commit(
                        self.repo,
                        info.id,
                        info.parent_ids.into_iter().collect(),
                        self.refs.get(&info.id).cloned().unwrap_or_default(),
                        self.shallow.contains(&info.id),
                        self.first_parent,
                    )?;
                    let row = self.layout.push(commit);
                    self.lanes = self.lanes.max(row.lanes);
                    self.store.push(&row)?;
                }
            }
        }
        Ok(())
    }
    fn find(&mut self, query: &Query, from: usize, backwards: bool, id: Option<&str>) -> Result<Option<usize>> {
        if backwards {
            for index in (0..from.min(self.store.len())).rev() {
                ensure!(!self.cancel.load(Ordering::Relaxed), "History cancelled");
                let row = self.store.read(index)?;
                if id.map_or_else(|| query.matches(&row.commit), |id| row.commit.id == id) {
                    return Ok(Some(index));
                }
            }
        } else {
            let mut index = from;
            loop {
                self.fill(index + 1)?;
                if index >= self.store.len() {
                    break;
                }
                ensure!(!self.cancel.load(Ordering::Relaxed), "History cancelled");
                let row = self.store.read(index)?;
                if id.map_or_else(|| query.matches(&row.commit), |id| row.commit.id == id) {
                    return Ok(Some(index));
                }
                index += 1;
            }
        }
        Ok(None)
    }
}
fn serve(
    root: &Path,
    scope: Scope,
    first_parent: bool,
    requests: mpsc::Receiver<Request>,
    events: &mpsc::Sender<Event>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let started = std::time::Instant::now();
    let mut repo = gix::discover(root)?;
    repo.object_cache_size_if_unset(8 * 1024 * 1024);
    let mut cursor = Cursor::with_options(&repo, scope, first_parent, cancel.clone())?;
    let mut refs: Vec<_> = cursor.refs.values().flatten().filter(|r| r.kind != RefKind::Head).cloned().collect();
    refs.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    events.send(Event::Ready { refs })?;
    tracing::debug!(elapsed_ms = started.elapsed().as_millis(), "History traversal initialized");
    // A timeout allows a cancelled worker to exit even when idle.
    while !cancel.load(Ordering::Relaxed) {
        let request = match requests.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        let event = match request {
            Request::Page { start, count } => {
                cursor.fill(start.saturating_add(count.min(MAX_CACHE)).saturating_add(PREFETCH))?;
                Event::Page {
                    start,
                    rows: cursor.store.page(start, count)?,
                    scanned: cursor.store.len(),
                    done: cursor.done,
                    lanes: cursor.lanes,
                }
            }
            Request::Find { query, from, backwards } => Event::Found {
                index: cursor.find(&query, from, backwards, None)?,
                scanned: cursor.store.len(),
                done: cursor.done,
                lanes: cursor.lanes,
            },
            Request::End => {
                cursor.fill(usize::MAX)?;
                Event::Found {
                    index: cursor.store.len().checked_sub(1),
                    scanned: cursor.store.len(),
                    done: cursor.done,
                    lanes: cursor.lanes,
                }
            }
            // A commit among the first `within` rows, to keep it selected after a reload.
            Request::Locate { id, within } => Event::Found {
                index: {
                    cursor.fill(within)?;
                    let end = cursor.store.len().min(within);
                    cursor.find(&Query::default(), end, true, Some(&id))?
                },
                scanned: cursor.store.len(),
                done: cursor.done,
                lanes: cursor.lanes,
            },
        };
        if events.send(event).is_err() {
            break;
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Details {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub message: String,
}
pub fn details(root: &Path, id: &str) -> Result<Details> {
    let repo = gix::discover(root)?;
    let oid = gix::ObjectId::from_hex(id.as_bytes())?;
    let object = repo.find_object(oid)?.try_into_commit()?;
    let decoded = object.decode()?;
    let author = decoded.author()?;
    Ok(Details {
        id: id.into(),
        parents: object.parent_ids().map(|p| p.to_string()).collect(),
        author: author.name.to_string(),
        email: author.email.to_string(),
        timestamp: decoded.committer()?.time()?.seconds,
        message: String::from_utf8_lossy(&decoded.message[..decoded.message.len().min(512 * 1024)]).into_owned(),
    })
}
