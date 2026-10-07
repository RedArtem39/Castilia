use alloc::{
    string::{String, ToString},
    vec::Vec,
};

pub type NodeId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element {
        tag: String,
        attributes: Vec<(String, String)>,
    },
    Text(String),
    Comment(String),
    /// A detached container produced by `createDocumentFragment` / `template` parsing.
    Fragment,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

impl Node {
    pub fn tag(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Element { tag, .. } => Some(tag),
            _ => None,
        }
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        match &self.kind {
            NodeKind::Element { attributes, .. } => attributes
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub head: NodeId,
    pub body: NodeId,
    /// True when the document has no (or a legacy) doctype: quirks mode.
    pub quirks: bool,
    /// Shadow hosts and their root fragments (shadow trees are not part of the light tree).
    pub shadow_roots: alloc::collections::BTreeMap<NodeId, NodeId>,
}

impl Document {
    pub fn create_element(&mut self, tag: &str) -> NodeId {
        self.push(
            NodeKind::Element {
                tag: tag.to_ascii_lowercase(),
                attributes: Vec::new(),
            },
            None,
        )
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> bool {
        if parent >= self.nodes.len() || child >= self.nodes.len() || child == self.root {
            return false;
        }
        if matches!(self.nodes[parent].kind, NodeKind::Text(_) | NodeKind::Comment(_)) {
            return false;
        }
        let mut current = Some(parent);
        while let Some(id) = current {
            if id == child {
                return false;
            }
            current = self.nodes[id].parent;
        }
        if let Some(old_parent) = self.nodes[child].parent {
            self.nodes[old_parent].children.retain(|&id| id != child);
        }
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
        true
    }

    /// Insert `child` into `parent` before `reference` (append when `None`).
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) -> bool {
        let Some(reference) = reference else {
            return self.append_child(parent, child);
        };
        if reference >= self.nodes.len() || self.nodes[reference].parent != Some(parent) || reference == child {
            return false;
        }
        // reuse append_child's validation, then move into position
        if !self.append_child(parent, child) {
            return false;
        }
        let list = &mut self.nodes[parent].children;
        list.retain(|&c| c != child);
        let at = list.iter().position(|&c| c == reference).unwrap_or(list.len());
        list.insert(at, child);
        true
    }

    /// Deep or shallow copy of `id` as a detached tree; returns the new root.
    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        let kind = self.nodes[id].kind.clone();
        let copy = self.push(kind, None);
        if deep {
            let kids = self.nodes[id].children.clone();
            for k in kids {
                let c = self.clone_node(k, true);
                self.nodes[c].parent = Some(copy);
                self.nodes[copy].children.push(c);
            }
        }
        copy
    }

    pub fn remove(&mut self, id: NodeId) -> bool {
        if id >= self.nodes.len() || id == self.root {
            return false;
        }
        let Some(parent) = self.nodes[id].parent.take() else {
            return false;
        };
        self.nodes[parent].children.retain(|&child| child != id);
        true
    }

    /// Find an element in the connected document tree.
    pub fn get_element_by_id(&self, value: &str) -> Option<NodeId> {
        let mut stack = Vec::from([self.root]);
        while let Some(id) = stack.pop() {
            if self.nodes[id].attribute("id") == Some(value) {
                return Some(id);
            }
            if self.nodes[id].tag() != Some("template") {
                stack.extend(self.nodes[id].children.iter().rev().copied());
            }
        }
        None
    }

    pub fn text_content(&self, id: NodeId) -> Option<String> {
        self.nodes.get(id)?;
        let mut result = String::new();
        let mut stack = Vec::from([id]);
        while let Some(next) = stack.pop() {
            match &self.nodes[next].kind {
                NodeKind::Text(text) => result.push_str(text),
                _ => stack.extend(self.nodes[next].children.iter().rev().copied()),
            }
        }
        Some(result)
    }

    pub fn is_connected(&self, id: NodeId) -> bool {
        let mut current = Some(id);
        while let Some(next) = current {
            if next == self.root {
                return true;
            }
            current = self.nodes.get(next).and_then(|node| node.parent);
        }
        false
    }

    pub fn set_text_content(&mut self, id: NodeId, value: &str) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        if matches!(node.kind, NodeKind::Document) {
            return false;
        }
        if let NodeKind::Text(text) | NodeKind::Comment(text) = &mut self.nodes[id].kind {
            *text = value.to_string();
            return true;
        }
        let old = core::mem::take(&mut self.nodes[id].children);
        for child in old {
            self.nodes[child].parent = None;
        }
        if !value.is_empty() {
            self.push(NodeKind::Text(value.to_string()), Some(id));
        }
        true
    }

    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        let NodeKind::Element { attributes, .. } = &mut node.kind else {
            return false;
        };
        let name = name.to_ascii_lowercase();
        if let Some((_, existing)) = attributes.iter_mut().find(|(key, _)| key == &name) {
            *existing = value.to_string();
        } else {
            attributes.push((name, value.to_string()));
        }
        true
    }

    pub(crate) fn new() -> Self {
        let mut document = Self {
            nodes: Vec::new(),
            root: 0,
            head: 0,
            body: 0,
            quirks: true,
            shadow_roots: alloc::collections::BTreeMap::new(),
        };
        document.root = document.push(NodeKind::Document, None);
        let html = document.push(
            NodeKind::Element {
                tag: String::from("html"),
                attributes: Vec::new(),
            },
            Some(document.root),
        );
        document.head = document.push(
            NodeKind::Element {
                tag: String::from("head"),
                attributes: Vec::new(),
            },
            Some(html),
        );
        document.body = document.push(
            NodeKind::Element {
                tag: String::from("body"),
                attributes: Vec::new(),
            },
            Some(html),
        );
        document
    }

    pub(crate) fn push(&mut self, kind: NodeKind, parent: Option<NodeId>) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            parent,
            children: Vec::new(),
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(id);
        }
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reparenting_keeps_one_parent_and_rejects_cycles() {
        let mut doc = Document::new();
        let first = doc.create_element("div");
        let second = doc.create_element("div");
        assert!(doc.append_child(doc.body, first));
        assert!(doc.append_child(doc.body, second));
        assert!(doc.append_child(first, second));
        assert_eq!(doc.nodes[second].parent, Some(first));
        assert!(!doc.nodes[doc.body].children.contains(&second));
        assert!(!doc.append_child(second, first));
        assert!(doc.is_connected(second));
    }
}
