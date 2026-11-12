use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
    Head,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decoration {
    pub name: String,
    pub full_name: String,
    pub kind: RefKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub refs: Vec<Decoration>,
    pub shallow: bool,
    /// Parents left out of a first-parent history (merged side branches).
    #[serde(default)]
    pub collapsed: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub top: u8,
    pub bottom: u8,
    pub color: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub commit: Commit,
    pub lane: usize,
    pub color: usize,
    pub lanes: usize,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    pub words: Vec<String>,
    pub author: Option<String>,
    pub after: Option<i64>,
    pub before: Option<i64>,
}
impl Query {
    /// Search keeps topology intact. Tokens narrow matches, never invent edges
    /// between commits whose parents have been filtered out.
    pub fn parse(input: &str) -> anyhow::Result<Self> {
        let mut query = Self::default();
        for token in input.split_whitespace() {
            if let Some(author) = token.strip_prefix("author:") {
                query.author = Some(author.to_lowercase());
            } else if let Some(date) = token.strip_prefix("after:") {
                query.after =
                    Some(gix::date::parse(date, None).map_err(|e| anyhow::anyhow!("Invalid date {date}: {e}"))?.seconds);
            } else if let Some(date) = token.strip_prefix("before:") {
                query.before =
                    Some(gix::date::parse(date, None).map_err(|e| anyhow::anyhow!("Invalid date {date}: {e}"))?.seconds + 86400);
            } else {
                query.words.push(token.to_lowercase());
            }
        }
        Ok(query)
    }
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.author.is_none() && self.after.is_none() && self.before.is_none()
    }
    pub fn matches(&self, c: &Commit) -> bool {
        let haystack = format!(
            "{} {} {} {} {}",
            c.id,
            c.subject,
            c.author,
            c.email,
            c.refs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(" ")
        )
        .to_lowercase();
        self.words.iter().all(|w| haystack.contains(w))
            && self.author.as_ref().is_none_or(|a| c.author.to_lowercase().contains(a) || c.email.to_lowercase().contains(a))
            && self.after.is_none_or(|time| c.timestamp >= time)
            && self.before.is_none_or(|time| c.timestamp < time)
    }
}
