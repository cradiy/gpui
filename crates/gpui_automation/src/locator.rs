use gpui::{AutomationNode, AutomationSnapshot, Role};
use std::{collections::HashMap, fmt, sync::Arc};

/// An immutable snapshot with indexed node lookup. It never refreshes implicitly.
pub struct Snapshot {
    data: Arc<AutomationSnapshot>,
    index: HashMap<u64, usize>,
}

impl Snapshot {
    pub fn new(data: Arc<AutomationSnapshot>) -> Self {
        let index = data
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();
        Self { data, index }
    }

    pub fn data(&self) -> &AutomationSnapshot {
        &self.data
    }

    pub fn node(&self, id: u64) -> Option<&AutomationNode> {
        self.index.get(&id).map(|index| &self.data.nodes[*index])
    }

    pub fn find(&self, selector: &Selector) -> Locator<'_> {
        Locator {
            snapshot: self,
            id: selector.id.clone(),
            role: selector.role,
            name: selector.name.clone(),
            scope: None,
            parent: selector.parent.clone(),
        }
    }

    pub fn get_by_id(&self, id: impl Into<String>) -> Locator<'_> {
        Locator {
            id: Some(id.into()),
            ..Locator::new(self)
        }
    }

    pub fn get_by_role(&self, role: Role) -> Locator<'_> {
        Locator {
            role: Some(role),
            ..Locator::new(self)
        }
    }

    pub fn get_by_name(&self, name: impl Into<String>) -> Locator<'_> {
        Locator::new(self).named(name)
    }
}

/// Exact-match query over one snapshot. Scope identifiers refer to that snapshot.
#[derive(Clone)]
pub struct Locator<'a> {
    snapshot: &'a Snapshot,
    id: Option<String>,
    role: Option<Role>,
    name: Option<String>,
    scope: Option<u64>,
    parent: Option<Box<Selector>>,
}

impl<'a> Locator<'a> {
    fn new(snapshot: &'a Snapshot) -> Self {
        Self {
            snapshot,
            id: None,
            role: None,
            name: None,
            scope: None,
            parent: None,
        }
    }

    /// Matches an explicitly authored semantic label, not visible-text inference.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Restricts matches to descendants of the given node, excluding the node itself.
    pub fn within(mut self, scope: u64) -> Self {
        self.scope = Some(scope);
        self
    }

    pub fn all(&self) -> Result<Vec<&'a AutomationNode>, LookupError> {
        let scope = match &self.parent {
            Some(parent) => Some(self.snapshot.find(parent).one()?.id),
            None => self.scope,
        };
        if let Some(scope) = scope
            && self.snapshot.node(scope).is_none()
        {
            return Err(LookupError::UnknownScope(scope));
        }
        Ok(self
            .snapshot
            .data
            .nodes
            .iter()
            .filter(|node| {
                if self
                    .id
                    .as_ref()
                    .is_some_and(|id| node.automation_id.as_ref() != Some(id))
                    || self.role.is_some_and(|role| node.role != role)
                    || self
                        .name
                        .as_ref()
                        .is_some_and(|name| node.label.as_ref() != Some(name))
                {
                    return false;
                }
                let Some(scope) = scope else {
                    return true;
                };
                let mut parent = node.parent;
                for _ in 0..self.snapshot.data.nodes.len() {
                    let Some(id) = parent else {
                        break;
                    };
                    if id == scope {
                        return true;
                    }
                    parent = self.snapshot.node(id).and_then(|node| node.parent);
                }
                false
            })
            .collect())
    }

    /// Requires exactly one match. Ambiguous queries never silently pick a node.
    pub fn one(&self) -> Result<&'a AutomationNode, LookupError> {
        let matches = self.all()?;
        match matches.len() {
            0 => Err(LookupError::NotFound),
            1 => Ok(matches[0]),
            count => Err(LookupError::Ambiguous { count }),
        }
    }
}

/// An owned query resolved anew against each snapshot. Parent scopes must also
/// resolve uniquely; no opaque node identity is retained across draws.
#[derive(Clone, Debug)]
pub struct Selector {
    id: Option<String>,
    role: Option<Role>,
    name: Option<String>,
    parent: Option<Box<Selector>>,
}

impl Selector {
    pub fn id(id: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
            role: None,
            name: None,
            parent: None,
        }
    }

    pub fn role(role: Role) -> Self {
        Self {
            id: None,
            role: Some(role),
            name: None,
            parent: None,
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn within(mut self, parent: Selector) -> Self {
        self.parent = Some(Box::new(parent));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupError {
    NotFound,
    Ambiguous { count: usize },
    UnknownScope(u64),
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("no element matches the locator"),
            Self::Ambiguous { count } => {
                write!(f, "locator matches {count} elements; narrow its scope")
            }
            Self::UnknownScope(id) => write!(f, "scope {id} is absent from this snapshot"),
        }
    }
}
impl std::error::Error for LookupError {}
