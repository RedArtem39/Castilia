//! Node / Element / Document / Text natives.
use super::{builtins::*, dom::*, interp::*, url::Url, value::*};
use crate::{
    dom::{NodeId, NodeKind},
    html, selector,
};
use alloc::{
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

pub fn setup_all(it: &mut Interp) {
    // prototypes that need to exist before the constructors
    let node = it.protos().node.clone();
    let element = it.protos().element.clone();
    let html_element = it.protos().html_element.clone();
    let text = it.protos().text.clone();
    let document = it.protos().document.clone();
    let obj = it.protos().object.clone();
    let comment = it.alloc(Object::new(Some(node.clone()), Class::Plain));
    let fragment = it.alloc(Object::new(Some(node.clone()), Class::Plain));
    let nodelist = it.alloc(Object::new(Some(it.protos().array.clone()), Class::Plain));
    let _ = obj;
    {
        let p = it.protos.as_mut().unwrap();
        p.extra.insert("comment", comment.clone());
        p.extra.insert("fragment", fragment.clone());
        p.extra.insert("nodelist", nodelist.clone());
    }
    fn illegal(it: &mut Interp, _c: &Call) -> Res {
        it.type_error("Illegal constructor")
    }
    let node_ctor = it.make_ctor("Node", 0, illegal, &node);
    it.make_ctor("Element", 0, illegal, &element);
    it.make_ctor("HTMLElement", 0, html_element_ctor, &html_element);
    it.make_ctor("Text", 0, |it, c| {
        let s = if c.args.is_empty() { Rc::from("") } else { it.to_str(&c.arg(0))? };
        let doc = it.dom()?;
        let id = doc.push(NodeKind::Text(s.to_string()), None);
        Ok(it.wrap(id))
    }, &text);
    it.make_ctor("Comment", 0, |it, c| {
        let s = if c.args.is_empty() { Rc::from("") } else { it.to_str(&c.arg(0))? };
        let doc = it.dom()?;
        let id = doc.push(NodeKind::Comment(s.to_string()), None);
        Ok(it.wrap(id))
    }, &comment);
    it.make_ctor("DocumentFragment", 0, |it, _| {
        let doc = it.dom()?;
        let id = doc.push(NodeKind::Fragment, None);
        Ok(it.wrap(id))
    }, &fragment);
    it.make_ctor("Document", 0, illegal, &document);
    it.make_ctor("HTMLDocument", 0, illegal, &document);
    it.make_ctor("NodeList", 0, illegal, &nodelist);
    it.make_ctor("HTMLCollection", 0, illegal, &nodelist);
    it.def_method(&nodelist, "item", 1, |it, c| {
        let i = to_integer(it.arg_num(c, 0)?);
        Ok(match &c.this {
            Value::Obj(o) => match &o.borrow().class {
                Class::Array(a) if i >= 0.0 => a.get(i as usize).cloned().map(unhole).unwrap_or(Value::Null),
                _ => Value::Null,
            },
            _ => Value::Null,
        })
    });
    it.def_method(&nodelist, "namedItem", 1, |it, c| {
        let n = it.arg_str(c, 0)?;
        let items = it.array_like_to_vec(&c.this)?;
        for v in items {
            let id = it.node_id(&v)?;
            if it.attr(id, "id").as_deref() == Some(&*n) || it.attr(id, "name").as_deref() == Some(&*n) {
                return Ok(v);
            }
        }
        Ok(Value::Null)
    });
    for (n, v) in [("ELEMENT_NODE", 1.0), ("ATTRIBUTE_NODE", 2.0), ("TEXT_NODE", 3.0), ("CDATA_SECTION_NODE", 4.0), ("PROCESSING_INSTRUCTION_NODE", 7.0), ("COMMENT_NODE", 8.0), ("DOCUMENT_NODE", 9.0), ("DOCUMENT_TYPE_NODE", 10.0), ("DOCUMENT_FRAGMENT_NODE", 11.0), ("DOCUMENT_POSITION_DISCONNECTED", 1.0), ("DOCUMENT_POSITION_PRECEDING", 2.0), ("DOCUMENT_POSITION_FOLLOWING", 4.0), ("DOCUMENT_POSITION_CONTAINS", 8.0), ("DOCUMENT_POSITION_CONTAINED_BY", 16.0)] {
        it.define_data(&node_ctor, Key::str(n), Value::Num(v), 0);
        it.define_data(&node, Key::str(n), Value::Num(v), 0);
    }
    // tag specific prototypes
    let tags: &[(&str, &str, &str)] = &[
        ("a", "HTMLAnchorElement", "a"),
        ("img", "HTMLImageElement", "img"),
        ("input", "HTMLInputElement", "input"),
        ("button", "HTMLButtonElement", "button"),
        ("form", "HTMLFormElement", "form"),
        ("select", "HTMLSelectElement", "select"),
        ("textarea", "HTMLTextAreaElement", "textarea"),
        ("option", "HTMLOptionElement", "option"),
        ("canvas", "HTMLCanvasElement", "canvas"),
        ("script", "HTMLScriptElement", "script"),
        ("style", "HTMLStyleElement", "style"),
        ("link", "HTMLLinkElement", "link"),
        ("iframe", "HTMLIFrameElement", "iframe"),
        ("div", "HTMLDivElement", "div"),
        ("span", "HTMLSpanElement", "span"),
        ("p", "HTMLParagraphElement", "p"),
        ("table", "HTMLTableElement", "table"),
        ("tr", "HTMLTableRowElement", "tr"),
        ("td", "HTMLTableCellElement", "td"),
        ("ul", "HTMLUListElement", "ul"),
        ("ol", "HTMLOListElement", "ol"),
        ("li", "HTMLLIElement", "li"),
        ("label", "HTMLLabelElement", "label"),
        ("video", "HTMLVideoElement", "video"),
        ("audio", "HTMLAudioElement", "audio"),
        ("body", "HTMLBodyElement", "body"),
        ("head", "HTMLHeadElement", "head"),
        ("html", "HTMLHtmlElement", "html"),
        ("svg", "SVGElement", "svg"),
        ("template", "HTMLTemplateElement", "template"),
        ("meta", "HTMLMetaElement", "meta"),
        ("title", "HTMLTitleElement", "title"),
        ("heading", "HTMLHeadingElement", "heading"),
    ];
    for (key, ctor_name, _) in tags {
        let parent = if *key == "svg" { element.clone() } else { html_element.clone() };
        let p = it.alloc(Object::new(Some(parent), Class::Plain));
        it.make_ctor(ctor_name, 0, illegal, &p);
        it.protos.as_mut().unwrap().extra.insert(key, p);
    }
    for extra in ["HTMLUnknownElement", "HTMLBRElement", "HTMLHRElement", "HTMLPreElement", "HTMLQuoteElement", "HTMLDetailsElement", "HTMLDialogElement", "HTMLFieldSetElement", "HTMLLegendElement", "HTMLMapElement", "HTMLMediaElement", "HTMLProgressElement", "HTMLSlotElement", "HTMLSourceElement", "HTMLTableSectionElement", "HTMLTableColElement", "HTMLTableCaptionElement", "HTMLTimeElement", "HTMLOptGroupElement", "HTMLOutputElement", "HTMLMeterElement", "HTMLDataElement", "HTMLEmbedElement", "HTMLObjectElement", "HTMLBaseElement", "SVGSVGElement", "SVGGraphicsElement", "ShadowRoot", "CharacterData", "Attr", "NamedNodeMap", "DOMTokenList", "CSSStyleDeclaration", "DOMRect", "Range", "Selection", "XMLDocument", "DocumentType", "CSSStyleSheet", "StyleSheet", "TreeWalker", "NodeIterator", "FormData_", "ValidityState", "File_", "XPathResult"] {
        let p = it.new_object();
        it.make_ctor(extra, 0, illegal, &p);
    }

    setup_node(it, &node);
    setup_parent_child_mixins(it, &[&element, &document, &fragment], &[&element, &text, &comment]);
    setup_element(it, &element);
    it.def_accessor(&fragment, "innerHTML", inner_html_get, inner_html_set);
    setup_char_data(it, &text, &comment);
    setup_document(it, &document, &fragment);
    super::dom_extra::setup(it);
}

fn html_element_ctor(it: &mut Interp, c: &Call) -> Res {
    // used by `class X extends HTMLElement` through customElements.define
    let Some(nt) = c.new_target.clone() else { return it.type_error("Illegal constructor") };
    let tag = it.custom_elements.iter().find(|(_, ctor)| ctor.ptr_eq(&nt)).map(|(t, _)| t.clone());
    let Some(tag) = tag else { return it.type_error("Illegal constructor: custom element not registered") };
    // upgrading an existing element: `super()` returns that node
    let id = match it.upgrade_target.take() {
        Some(id) => id,
        None => it.dom()?.create_element(&tag),
    };
    it.upgraded.insert(id);
    let w = it.wrap(id);
    if let Value::Obj(o) = &w {
        let p = it.get_obj(&nt, &it.names.prototype.clone(), &Value::Obj(nt.clone()))?;
        if let Value::Obj(p) = p {
            o.borrow_mut().proto = Some(p);
        }
    }
    Ok(w)
}

fn node_kind(it: &mut Interp, id: NodeId) -> NodeKind {
    it.doc().map(|d| d.nodes[id].kind.clone()).unwrap_or(NodeKind::Fragment)
}

pub fn node_type(k: &NodeKind) -> f64 {
    match k {
        NodeKind::Element { .. } => 1.0,
        NodeKind::Text(_) => 3.0,
        NodeKind::Comment(_) => 8.0,
        NodeKind::Document => 9.0,
        NodeKind::Fragment => 11.0,
    }
}

pub fn upper_tag(tag: &str) -> String {
    tag.to_ascii_uppercase()
}

fn opt_wrap(it: &mut Interp, id: Option<NodeId>) -> Value {
    match id {
        Some(i) => it.wrap(i),
        None => Value::Null,
    }
}

fn siblings(it: &mut Interp, id: NodeId) -> (Vec<NodeId>, usize) {
    let Some(d) = it.doc() else { return (Vec::new(), 0) };
    match d.nodes[id].parent {
        Some(p) => {
            let list = d.nodes[p].children.clone();
            let pos = list.iter().position(|&c| c == id).unwrap_or(0);
            (list, pos)
        }
        None => (vec![id], 0),
    }
}

fn contains(doc: &crate::dom::Document, ancestor: NodeId, mut n: NodeId) -> bool {
    loop {
        if n == ancestor {
            return true;
        }
        match doc.nodes[n].parent {
            Some(p) => n = p,
            None => return false,
        }
    }
}

fn setup_node(it: &mut Interp, p: &Obj) {
    it.def_getter(p, "nodeType", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::Num(node_type(&node_kind(it, id))))
    });
    it.def_getter(p, "nodeName", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::string(match node_kind(it, id) {
            NodeKind::Element { tag, .. } => upper_tag(&tag),
            NodeKind::Text(_) => "#text".into(),
            NodeKind::Comment(_) => "#comment".into(),
            NodeKind::Document => "#document".into(),
            NodeKind::Fragment => "#document-fragment".into(),
        }))
    });
    it.def_accessor(
        p,
        "nodeValue",
        |it, c| {
            let id = it.this_node(c)?;
            Ok(match node_kind(it, id) {
                NodeKind::Text(t) | NodeKind::Comment(t) => Value::string(t),
                _ => Value::Null,
            })
        },
        |it, c| {
            let id = it.this_node(c)?;
            let s = it.arg_str(c, 0)?;
            if matches!(node_kind(it, id), NodeKind::Text(_) | NodeKind::Comment(_)) {
                it.dom_set_text(id, &s)?;
            }
            Ok(Value::Undefined)
        },
    );
    it.def_accessor(p, "textContent", text_content_get, text_content_set);
    it.def_getter(p, "parentNode", |it, c| {
        let id = it.this_node(c)?;
        let p = it.dom()?.nodes[id].parent;
        Ok(opt_wrap(it, p))
    });
    it.def_getter(p, "parentElement", |it, c| {
        let id = it.this_node(c)?;
        let p = it.dom()?.nodes[id].parent.filter(|&p| matches!(it.doc().map(|d| &d.nodes[p].kind), Some(NodeKind::Element { .. })));
        Ok(opt_wrap(it, p))
    });
    it.def_getter(p, "childNodes", |it, c| {
        let id = it.this_node(c)?;
        let kids = it.dom()?.nodes[id].children.clone();
        Ok(it.node_list(&kids))
    });
    it.def_getter(p, "firstChild", |it, c| {
        let id = it.this_node(c)?;
        let k = it.dom()?.nodes[id].children.first().copied();
        Ok(opt_wrap(it, k))
    });
    it.def_getter(p, "lastChild", |it, c| {
        let id = it.this_node(c)?;
        let k = it.dom()?.nodes[id].children.last().copied();
        Ok(opt_wrap(it, k))
    });
    it.def_getter(p, "nextSibling", |it, c| {
        let id = it.this_node(c)?;
        let (l, pos) = siblings(it, id);
        Ok(opt_wrap(it, l.get(pos + 1).copied().filter(|_| it.doc().is_some_and(|d| d.nodes[id].parent.is_some()))))
    });
    it.def_getter(p, "previousSibling", |it, c| {
        let id = it.this_node(c)?;
        let (l, pos) = siblings(it, id);
        Ok(opt_wrap(it, if pos > 0 { l.get(pos - 1).copied() } else { None }))
    });
    it.def_getter(p, "ownerDocument", |it, c| {
        let id = it.this_node(c)?;
        if matches!(node_kind(it, id), NodeKind::Document) {
            return Ok(Value::Null);
        }
        let root = it.dom()?.root;
        Ok(it.wrap(root))
    });
    it.def_getter(p, "isConnected", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::Bool(it.dom()?.is_connected(id)))
    });
    it.def_getter(p, "baseURI", |it, _| Ok(Value::string(it.dom.base_url.clone())));
    it.def_method(p, "appendChild", 1, |it, c| {
        let parent = it.this_node(c)?;
        let child = it.node_id(&c.arg(0))?;
        check_insert(it, parent, child)?;
        it.dom_insert(parent, child, None)?;
        Ok(c.arg(0))
    });
    it.def_method(p, "insertBefore", 2, |it, c| {
        let parent = it.this_node(c)?;
        let child = it.node_id(&c.arg(0))?;
        let before = it.opt_node(&c.arg(1))?;
        check_insert(it, parent, child)?;
        if let Some(b) = before {
            if it.dom()?.nodes[b].parent != Some(parent) {
                return Err(it.error_ctl("NotFoundError", "The node before which the new node is to be inserted is not a child of this node."));
            }
        }
        it.dom_insert(parent, child, before)?;
        Ok(c.arg(0))
    });
    it.def_method(p, "removeChild", 1, |it, c| {
        let parent = it.this_node(c)?;
        let child = it.node_id(&c.arg(0))?;
        if it.dom()?.nodes[child].parent != Some(parent) {
            return Err(it.error_ctl("NotFoundError", "The node to be removed is not a child of this node."));
        }
        it.dom_remove(child)?;
        Ok(c.arg(0))
    });
    it.def_method(p, "replaceChild", 2, |it, c| {
        let parent = it.this_node(c)?;
        let new = it.node_id(&c.arg(0))?;
        let old = it.node_id(&c.arg(1))?;
        if it.dom()?.nodes[old].parent != Some(parent) {
            return Err(it.error_ctl("NotFoundError", "The node to be replaced is not a child of this node."));
        }
        check_insert(it, parent, new)?;
        let next = {
            let (l, pos) = siblings(it, old);
            l.get(pos + 1).copied()
        };
        it.dom_remove(old)?;
        let next = next.filter(|&n| n != new);
        it.dom_insert(parent, new, next)?;
        Ok(c.arg(1))
    });
    it.def_method(p, "cloneNode", 1, |it, c| {
        let id = it.this_node(c)?;
        let deep = c.arg(0).truthy();
        let copy = it.dom()?.clone_node(id, deep);
        Ok(it.wrap(copy))
    });
    it.def_method(p, "contains", 1, |it, c| {
        let id = it.this_node(c)?;
        let Some(other) = it.opt_node(&c.arg(0))? else { return Ok(Value::Bool(false)) };
        Ok(Value::Bool(contains(it.dom()?, id, other)))
    });
    it.def_method(p, "hasChildNodes", 0, |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::Bool(!it.dom()?.nodes[id].children.is_empty()))
    });
    it.def_method(p, "getRootNode", 0, |it, c| {
        let mut id = it.this_node(c)?;
        while let Some(p) = it.dom()?.nodes[id].parent {
            id = p;
        }
        Ok(it.wrap(id))
    });
    it.def_method(p, "normalize", 0, |_, _| Ok(Value::Undefined));
    it.def_method(p, "isSameNode", 1, |it, c| Ok(Value::Bool(it.opt_node(&c.arg(0))? == Some(it.this_node(c)?))));
    it.def_method(p, "isEqualNode", 1, |it, c| {
        let a = it.this_node(c)?;
        let Some(b) = it.opt_node(&c.arg(0))? else { return Ok(Value::Bool(false)) };
        let d = it.dom()?;
        Ok(Value::Bool(html::serialize_outer(d, a) == html::serialize_outer(d, b)))
    });
    it.def_method(p, "compareDocumentPosition", 1, |it, c| {
        let a = it.this_node(c)?;
        let b = it.node_id(&c.arg(0))?;
        if a == b {
            return Ok(Value::Num(0.0));
        }
        let d = it.dom()?;
        if contains(d, a, b) {
            return Ok(Value::Num(20.0));
        }
        if contains(d, b, a) {
            return Ok(Value::Num(10.0));
        }
        // document order via DFS
        let mut order = Vec::new();
        let mut st = vec![d.root];
        while let Some(n) = st.pop() {
            order.push(n);
            st.extend(d.nodes[n].children.iter().rev().copied());
        }
        let pa = order.iter().position(|&x| x == a);
        let pb = order.iter().position(|&x| x == b);
        Ok(Value::Num(match (pa, pb) {
            (Some(x), Some(y)) => if y > x { 4.0 } else { 2.0 },
            _ => 1.0,
        }))
    });
    it.def_method(p, "lookupNamespaceURI", 1, |_, _| Ok(Value::Null));
}

fn check_insert(it: &mut Interp, parent: NodeId, child: NodeId) -> Res<()> {
    let d = it.dom()?;
    if matches!(d.nodes[parent].kind, NodeKind::Text(_) | NodeKind::Comment(_)) {
        return Err(it.error_ctl("HierarchyRequestError", "This node type does not support this method."));
    }
    if contains(d, child, parent) {
        return Err(it.error_ctl("HierarchyRequestError", "The new child element contains the parent."));
    }
    Ok(())
}

fn text_content_get(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let d = it.dom()?;
    Ok(match &d.nodes[id].kind {
        NodeKind::Document => Value::Null,
        NodeKind::Text(t) | NodeKind::Comment(t) => Value::str(t),
        _ => Value::string(d.text_content(id).unwrap_or_default()),
    })
}

fn text_content_set(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let s = if c.arg(0).is_nullish() { Rc::from("") } else { it.arg_str(c, 0)? };
    if matches!(node_kind(it, id), NodeKind::Document) {
        return Ok(Value::Undefined);
    }
    it.dom_set_text(id, &s)?;
    Ok(Value::Undefined)
}

/// Resolve `Node | string` arguments into nodes (strings become Text nodes).
fn nodes_from_args(it: &mut Interp, args: &[Value]) -> Res<Vec<NodeId>> {
    let mut out = Vec::new();
    for a in args {
        if let Value::Obj(o) = a {
            if let Class::Node(id) = o.borrow().class {
                out.push(id);
                continue;
            }
        }
        let s = it.to_str(a)?;
        let id = it.dom()?.push(NodeKind::Text(s.to_string()), None);
        out.push(id);
    }
    Ok(out)
}

fn element_children(it: &mut Interp, id: NodeId) -> Vec<NodeId> {
    let Some(d) = it.doc() else { return Vec::new() };
    d.nodes[id].children.iter().copied().filter(|&c| matches!(d.nodes[c].kind, NodeKind::Element { .. })).collect()
}

fn setup_parent_child_mixins(it: &mut Interp, parents: &[&Obj], children: &[&Obj]) {
    for p in parents {
        it.def_getter(p, "children", |it, c| {
            let id = it.this_node(c)?;
            let kids = element_children(it, id);
            Ok(it.node_list(&kids))
        });
        it.def_getter(p, "firstElementChild", |it, c| {
            let id = it.this_node(c)?;
            let k = element_children(it, id).first().copied();
            Ok(opt_wrap(it, k))
        });
        it.def_getter(p, "lastElementChild", |it, c| {
            let id = it.this_node(c)?;
            let k = element_children(it, id).last().copied();
            Ok(opt_wrap(it, k))
        });
        it.def_getter(p, "childElementCount", |it, c| {
            let id = it.this_node(c)?;
            Ok(Value::Num(element_children(it, id).len() as f64))
        });
        it.def_method(p, "querySelector", 1, |it, c| {
            let id = it.this_node(c)?;
            let s = it.arg_str(c, 0)?;
            let r = it.query(id, &s, true)?;
            Ok(opt_wrap(it, r.first().copied()))
        });
        it.def_method(p, "querySelectorAll", 1, |it, c| {
            let id = it.this_node(c)?;
            let s = it.arg_str(c, 0)?;
            let r = it.query(id, &s, false)?;
            Ok(it.node_list(&r))
        });
        it.def_method(p, "append", 1, |it, c| {
            let id = it.this_node(c)?;
            for n in nodes_from_args(it, c.args)? {
                it.dom_insert(id, n, None)?;
            }
            Ok(Value::Undefined)
        });
        it.def_method(p, "prepend", 1, |it, c| {
            let id = it.this_node(c)?;
            let first = it.dom()?.nodes[id].children.first().copied();
            for n in nodes_from_args(it, c.args)? {
                it.dom_insert(id, n, first)?;
            }
            Ok(Value::Undefined)
        });
        it.def_method(p, "replaceChildren", 0, |it, c| {
            let id = it.this_node(c)?;
            let nodes = nodes_from_args(it, c.args)?;
            let old = it.dom()?.nodes[id].children.clone();
            for o in old {
                it.dom_remove(o)?;
            }
            for n in nodes {
                it.dom_insert(id, n, None)?;
            }
            Ok(Value::Undefined)
        });
        it.def_method(p, "getElementById", 1, |it, c| {
            let id = it.this_node(c)?;
            let want = it.arg_str(c, 0)?;
            let d = it.dom()?;
            let mut st: Vec<NodeId> = d.nodes[id].children.iter().rev().copied().collect();
            while let Some(n) = st.pop() {
                if d.nodes[n].attribute("id") == Some(&*want) {
                    return Ok(it.wrap(n));
                }
                if t_not_template(d, n) {
                    st.extend(d.nodes[n].children.iter().rev().copied());
                }
            }
            Ok(Value::Null)
        });
    }
    for p in children {
        it.def_method(p, "remove", 0, |it, c| {
            let id = it.this_node(c)?;
            it.dom_remove(id)?;
            Ok(Value::Undefined)
        });
        it.def_method(p, "before", 1, |it, c| {
            let id = it.this_node(c)?;
            let Some(parent) = it.dom()?.nodes[id].parent else { return Ok(Value::Undefined) };
            for n in nodes_from_args(it, c.args)? {
                it.dom_insert(parent, n, Some(id))?;
            }
            Ok(Value::Undefined)
        });
        it.def_method(p, "after", 1, |it, c| {
            let id = it.this_node(c)?;
            let Some(parent) = it.dom()?.nodes[id].parent else { return Ok(Value::Undefined) };
            let (l, pos) = siblings(it, id);
            let mut next = l.get(pos + 1).copied();
            let nodes = nodes_from_args(it, c.args)?;
            while next.is_some_and(|n| nodes.contains(&n)) {
                let (l, pos) = siblings(it, next.unwrap());
                next = l.get(pos + 1).copied();
            }
            for n in nodes {
                it.dom_insert(parent, n, next)?;
            }
            Ok(Value::Undefined)
        });
        it.def_method(p, "replaceWith", 1, |it, c| {
            let id = it.this_node(c)?;
            let Some(parent) = it.dom()?.nodes[id].parent else { return Ok(Value::Undefined) };
            let (l, pos) = siblings(it, id);
            let mut next = l.get(pos + 1).copied();
            let nodes = nodes_from_args(it, c.args)?;
            while next.is_some_and(|n| nodes.contains(&n)) {
                let (l, pos) = siblings(it, next.unwrap());
                next = l.get(pos + 1).copied();
            }
            it.dom_remove(id)?;
            for n in nodes {
                it.dom_insert(parent, n, next)?;
            }
            Ok(Value::Undefined)
        });
        it.def_getter(p, "nextElementSibling", |it, c| {
            let id = it.this_node(c)?;
            let (l, pos) = siblings(it, id);
            let n = l.iter().skip(pos + 1).copied().find(|&n| it.is_element(n));
            Ok(opt_wrap(it, n))
        });
        it.def_getter(p, "previousElementSibling", |it, c| {
            let id = it.this_node(c)?;
            let (l, pos) = siblings(it, id);
            let n = l.iter().take(pos).rev().copied().find(|&n| it.is_element(n));
            Ok(opt_wrap(it, n))
        });
    }
}

// ---------------------------------------------------------------------------
// Element
// ---------------------------------------------------------------------------

fn this_el(it: &mut Interp, c: &Call) -> Res<NodeId> {
    let id = it.this_node(c)?;
    if !it.is_element(id) {
        return it.type_error("Illegal invocation: not an element");
    }
    Ok(id)
}

fn attrs_of(it: &mut Interp, id: NodeId) -> Vec<(String, String)> {
    match it.doc().map(|d| &d.nodes[id].kind) {
        Some(NodeKind::Element { attributes, .. }) => attributes.clone(),
        _ => Vec::new(),
    }
}

fn setup_element(it: &mut Interp, p: &Obj) {
    it.def_getter(p, "tagName", |it, c| {
        let id = this_el(it, c)?;
        let t = it.tag_name(id);
        Ok(Value::string(if t.contains(':') || is_svg_tag(&t) { t } else { upper_tag(&t) }))
    });
    it.def_getter(p, "localName", |it, c| {
        let id = this_el(it, c)?;
        Ok(Value::string(it.tag_name(id)))
    });
    it.def_getter(p, "namespaceURI", |it, c| {
        let id = this_el(it, c)?;
        let t = it.tag_name(id);
        Ok(Value::str(if t == "svg" || is_svg_tag(&t) { "http://www.w3.org/2000/svg" } else { "http://www.w3.org/1999/xhtml" }))
    });
    it.def_getter(p, "prefix", |_, _| Ok(Value::Null));
    it.def_method(p, "getAttribute", 1, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 0)?.to_ascii_lowercase();
        Ok(it.attr(id, &n).map_or(Value::Null, Value::string))
    });
    it.def_method(p, "getAttributeNS", 2, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 1)?.to_ascii_lowercase();
        Ok(it.attr(id, &n).map_or(Value::Null, Value::string))
    });
    it.def_method(p, "setAttribute", 2, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 0)?;
        let v = it.arg_str(c, 1)?;
        it.dom_set_attr(id, &n, &v)?;
        it.attribute_changed(id, &n)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "setAttributeNS", 3, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 1)?;
        let v = it.arg_str(c, 2)?;
        it.dom_set_attr(id, &n, &v)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "removeAttribute", 1, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 0)?;
        it.dom_remove_attr(id, &n)?;
        it.attribute_changed(id, &n)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "removeAttributeNS", 2, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 1)?;
        it.dom_remove_attr(id, &n)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "hasAttribute", 1, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 0)?.to_ascii_lowercase();
        Ok(Value::Bool(it.attr(id, &n).is_some()))
    });
    it.def_method(p, "hasAttributeNS", 2, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 1)?.to_ascii_lowercase();
        Ok(Value::Bool(it.attr(id, &n).is_some()))
    });
    it.def_method(p, "hasAttributes", 0, |it, c| {
        let id = this_el(it, c)?;
        Ok(Value::Bool(!attrs_of(it, id).is_empty()))
    });
    it.def_method(p, "toggleAttribute", 1, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 0)?;
        let has = it.attr(id, &n.to_ascii_lowercase()).is_some();
        let want = if c.args.len() > 1 { c.arg(1).truthy() } else { !has };
        if want && !has {
            it.dom_set_attr(id, &n, "")?;
        } else if !want && has {
            it.dom_remove_attr(id, &n)?;
        }
        Ok(Value::Bool(want))
    });
    it.def_method(p, "getAttributeNames", 0, |it, c| {
        let id = this_el(it, c)?;
        let names: Vec<Value> = attrs_of(it, id).into_iter().map(|(k, _)| Value::string(k)).collect();
        Ok(it.new_array(names))
    });
    it.def_method(p, "getAttributeNode", 1, |it, c| {
        let id = this_el(it, c)?;
        let n = it.arg_str(c, 0)?.to_ascii_lowercase();
        Ok(match it.attr(id, &n) {
            Some(v) => make_attr(it, id, &n, &v),
            None => Value::Null,
        })
    });
    it.def_getter(p, "attributes", |it, c| {
        let id = this_el(it, c)?;
        let list: Vec<Value> = attrs_of(it, id).into_iter().map(|(k, v)| make_attr(it, id, &k, &v)).collect();
        let arr = it.new_array(list.clone());
        if let Value::Obj(o) = &arr {
            for (i, (k, _)) in attrs_of(it, id).into_iter().enumerate() {
                it.define_data(o, Key::S(Rc::from(k.as_str())), list[i].clone(), HIDDEN);
            }
            it.def_method(o, "getNamedItem", 1, |it, c| {
                let n = it.arg_str(c, 0)?;
                it.get_v(&c.this, &Key::S(n))
            });
        }
        Ok(arr)
    });
    it.def_accessor(p, "innerHTML", inner_html_get, inner_html_set);
    it.def_accessor(
        p,
        "outerHTML",
        |it, c| {
            let id = this_el(it, c)?;
            Ok(Value::string(html::serialize_outer(it.dom()?, id)))
        },
        |it, c| {
            let id = this_el(it, c)?;
            let s = it.arg_str(c, 0)?;
            let Some(parent) = it.dom()?.nodes[id].parent else { return Ok(Value::Undefined) };
            let (l, pos) = siblings(it, id);
            let next = l.get(pos + 1).copied();
            let frag = it.dom()?.push(NodeKind::Fragment, None);
            html::parse_fragment(it.dom()?, frag, &s);
            it.dom_remove(id)?;
            it.dom_insert(parent, frag, next)?;
            Ok(Value::Undefined)
        },
    );
    it.def_method(p, "insertAdjacentHTML", 2, |it, c| {
        let id = this_el(it, c)?;
        let pos = it.arg_str(c, 0)?.to_ascii_lowercase();
        let s = it.arg_str(c, 1)?;
        let frag = it.dom()?.push(NodeKind::Fragment, None);
        html::parse_fragment(it.dom()?, frag, &s);
        insert_adjacent(it, id, &pos, frag)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "insertAdjacentElement", 2, |it, c| {
        let id = this_el(it, c)?;
        let pos = it.arg_str(c, 0)?.to_ascii_lowercase();
        let n = it.node_id(&c.arg(1))?;
        insert_adjacent(it, id, &pos, n)?;
        Ok(c.arg(1))
    });
    it.def_method(p, "insertAdjacentText", 2, |it, c| {
        let id = this_el(it, c)?;
        let pos = it.arg_str(c, 0)?.to_ascii_lowercase();
        let s = it.arg_str(c, 1)?;
        let n = it.dom()?.push(NodeKind::Text(s.to_string()), None);
        insert_adjacent(it, id, &pos, n)?;
        Ok(Value::Undefined)
    });
    for name in ["matches", "webkitMatchesSelector", "msMatchesSelector", "mozMatchesSelector"] {
        it.def_method(p, name, 1, |it, c| {
            let id = this_el(it, c)?;
            let s = it.arg_str(c, 0)?;
            let list = it.selector(&s)?;
            Ok(Value::Bool(selector::matches_list(it.dom()?, id, &list)))
        });
    }
    it.def_method(p, "closest", 1, |it, c| {
        let id = this_el(it, c)?;
        let s = it.arg_str(c, 0)?;
        let list = it.selector(&s)?;
        let d = it.dom()?;
        let mut cur = Some(id);
        while let Some(n) = cur {
            if matches!(d.nodes[n].kind, NodeKind::Element { .. }) && selector::matches_list(d, n, &list) {
                return Ok(it.wrap(n));
            }
            cur = d.nodes[n].parent;
        }
        Ok(Value::Null)
    });
    it.def_method(p, "getElementsByTagName", 1, |it, c| {
        let id = this_el(it, c)?;
        let t = it.arg_str(c, 0)?.to_ascii_lowercase();
        let r = by_tag(it, id, &t);
        Ok(it.node_list(&r))
    });
    it.def_method(p, "getElementsByClassName", 1, |it, c| {
        let id = this_el(it, c)?;
        let s = it.arg_str(c, 0)?;
        let r = by_class(it, id, &s);
        Ok(it.node_list(&r))
    });
    it.def_method(p, "attachShadow", 1, |it, c| {
        let id = this_el(it, c)?;
        if it.dom()?.shadow_roots.contains_key(&id) {
            return Err(it.error_ctl("NotSupportedError", "Shadow root cannot be created on a host which already hosts a shadow tree."));
        }
        let mode = if matches!(c.arg(0), Value::Obj(_)) { it.get_str(&c.arg(0), "mode")? } else { Value::Undefined };
        let mode = it.to_string_lossy(&mode);
        let frag = it.dom()?.push(NodeKind::Fragment, None);
        it.dom()?.shadow_roots.insert(id, frag);
        if mode == "closed" {
            it.shadow_closed.push(id);
        }
        let w = it.wrap(frag);
        if let Value::Obj(o) = &w {
            let host = it.wrap(id);
            it.define_data(o, Key::str("host"), host, 0);
            it.define_data(o, Key::str("mode"), Value::string(if mode == "closed" { mode.clone() } else { "open".to_string() }), 0);
            it.dom.dirty = true;
        it.dom.version += 1;
        }
        Ok(w)
    });
    it.def_getter(p, "shadowRoot", |it, c| {
        let id = this_el(it, c)?;
        if it.shadow_closed.contains(&id) {
            return Ok(Value::Null);
        }
        let f = it.dom()?.shadow_roots.get(&id).copied();
        Ok(match f {
            Some(f) => it.wrap(f),
            None => Value::Null,
        })
    });
    it.def_method(p, "assignedNodes", 1, |it, c| {
        let id = this_el(it, c)?;
        let list = slot_assigned(it, id)?;
        Ok(it.node_list(&list))
    });
    it.def_method(p, "assignedElements", 1, |it, c| {
        let id = this_el(it, c)?;
        let mut list = slot_assigned(it, id)?;
        list.retain(|&n| it.is_element(n));
        Ok(it.node_list(&list))
    });
    it.def_method(p, "scrollIntoView", 0, |_, _| Ok(Value::Undefined));
    for n in ["scrollTo", "scroll", "scrollBy", "requestFullscreen", "setPointerCapture", "releasePointerCapture", "scrollIntoViewIfNeeded"] {
        it.def_method(p, n, 0, |_, _| Ok(Value::Undefined));
    }
    it.def_method(p, "checkVisibility", 0, |_, _| Ok(Value::Bool(true)));
    it.def_method(p, "getAnimations", 0, |it, _| Ok(it.new_array(Vec::new())));
    it.def_method(p, "animate", 2, |it, _| {
        let o = it.new_object();
        let pr = it.promise_resolved(Value::Undefined)?;
        it.define_data(&o, Key::str("finished"), Value::Obj(pr), ALL);
        for n in ["cancel", "finish", "play", "pause", "reverse"] {
            it.def_method(&o, n, 0, |_, _| Ok(Value::Undefined));
        }
        Ok(Value::Obj(o))
    });
    it.def_method(p, "getBoundingClientRect", 0, |it, c| {
        let id = this_el(it, c)?;
        let r = it.dom.rects.get(&id).copied().unwrap_or([0.0; 4]);
        Ok(make_rect(it, r[0] as f64 - it.dom.scroll.0 as f64, r[1] as f64 - it.dom.scroll.1 as f64, r[2] as f64, r[3] as f64))
    });
    it.def_method(p, "getClientRects", 0, |it, c| {
        let id = this_el(it, c)?;
        match it.dom.rects.get(&id).copied() {
            Some(r) => {
                let rect = make_rect(it, r[0] as f64, r[1] as f64 - it.dom.scroll.1 as f64, r[2] as f64, r[3] as f64);
                Ok(it.new_array(vec![rect]))
            }
            None => Ok(it.new_array(Vec::new())),
        }
    });
    macro_rules! dim {
        ($name:expr, $idx:expr) => {
            it.def_getter(p, $name, |it, c| {
                let id = this_el(it, c)?;
                let r = it.dom.rects.get(&id).copied().unwrap_or([0.0; 4]);
                Ok(Value::Num(crate::math::round(r[$idx] as f64)))
            });
        };
    }
    dim!("offsetLeft", 0);
    dim!("offsetTop", 1);
    dim!("offsetWidth", 2);
    dim!("offsetHeight", 3);
    dim!("clientWidth", 2);
    dim!("clientHeight", 3);
    dim!("scrollWidth", 2);
    dim!("scrollHeight", 3);
    for n in ["clientLeft", "clientTop"] {
        it.def_getter(p, n, |_, _| Ok(Value::Num(0.0)));
    }
    for n in ["scrollTop", "scrollLeft"] {
        it.def_accessor(p, n, |_, _| Ok(Value::Num(0.0)), |_, _| Ok(Value::Undefined));
    }
    it.def_getter(p, "offsetParent", |it, c| {
        let id = this_el(it, c)?;
        let p = it.dom()?.nodes[id].parent;
        Ok(opt_wrap(it, p))
    });
}

fn is_svg_tag(t: &str) -> bool {
    matches!(t, "circle" | "path" | "rect" | "g" | "line" | "polyline" | "polygon" | "ellipse" | "defs" | "use" | "symbol" | "text" | "tspan" | "lineargradient" | "radialgradient" | "stop" | "clippath" | "mask" | "pattern" | "foreignObject" | "image")
}

pub fn make_rect(it: &mut Interp, x: f64, y: f64, w: f64, h: f64) -> Value {
    let o = it.new_object();
    for (k, v) in [("x", x), ("y", y), ("width", w), ("height", h), ("top", y), ("left", x), ("right", x + w), ("bottom", y + h)] {
        it.define_data(&o, Key::str(k), Value::Num(v), ALL);
    }
    it.def_method(&o, "toJSON", 0, |it, c| {
        let r = it.new_object();
        for k in ["x", "y", "width", "height", "top", "left", "right", "bottom"] {
            let v = it.get_str(&c.this, k)?;
            it.define_data(&r, Key::str(k), v, ALL);
        }
        Ok(Value::Obj(r))
    });
    Value::Obj(o)
}

fn make_attr(it: &mut Interp, owner: NodeId, name: &str, value: &str) -> Value {
    let o = it.new_object();
    it.define_data(&o, Key::str("name"), Value::str(name), ALL);
    it.define_data(&o, Key::str("localName"), Value::str(name), ALL);
    it.define_data(&o, Key::str("nodeName"), Value::str(name), ALL);
    it.define_data(&o, Key::str("value"), Value::str(value), ALL);
    it.define_data(&o, Key::str("nodeValue"), Value::str(value), ALL);
    it.define_data(&o, Key::str("textContent"), Value::str(value), ALL);
    it.define_data(&o, Key::str("nodeType"), Value::Num(2.0), ALL);
    it.define_data(&o, Key::str("specified"), Value::Bool(true), ALL);
    let owner_v = it.wrap(owner);
    it.define_data(&o, Key::str("ownerElement"), owner_v, ALL);
    Value::Obj(o)
}

pub fn t_not_template(d: &crate::dom::Document, n: NodeId) -> bool {
    d.nodes[n].tag() != Some("template")
}

fn by_tag(it: &mut Interp, root: NodeId, tag: &str) -> Vec<NodeId> {
    let Some(d) = it.doc() else { return Vec::new() };
    let mut out = Vec::new();
    let mut st: Vec<NodeId> = d.nodes[root].children.iter().rev().copied().collect();
    while let Some(n) = st.pop() {
        if let Some(t) = d.nodes[n].tag() {
            if tag == "*" || t.eq_ignore_ascii_case(tag) {
                out.push(n);
            }
        }
        if t_not_template(d, n) {
            st.extend(d.nodes[n].children.iter().rev().copied());
        }
    }
    out
}

fn by_class(it: &mut Interp, root: NodeId, classes: &str) -> Vec<NodeId> {
    let Some(d) = it.doc() else { return Vec::new() };
    let want: Vec<&str> = classes.split_whitespace().collect();
    let mut out = Vec::new();
    if want.is_empty() {
        return out;
    }
    let mut st: Vec<NodeId> = d.nodes[root].children.iter().rev().copied().collect();
    while let Some(n) = st.pop() {
        if let Some(cl) = d.nodes[n].attribute("class") {
            let have: Vec<&str> = cl.split_whitespace().collect();
            if want.iter().all(|w| have.contains(w)) {
                out.push(n);
            }
        }
        if t_not_template(d, n) {
            st.extend(d.nodes[n].children.iter().rev().copied());
        }
    }
    out
}

fn insert_adjacent(it: &mut Interp, id: NodeId, pos: &str, node: NodeId) -> Res<()> {
    match pos {
        "beforebegin" => {
            if let Some(p) = it.dom()?.nodes[id].parent {
                it.dom_insert(p, node, Some(id))?;
            }
        }
        "afterbegin" => {
            let first = it.dom()?.nodes[id].children.first().copied();
            it.dom_insert(id, node, first)?;
        }
        "beforeend" => it.dom_insert(id, node, None)?,
        "afterend" => {
            if let Some(p) = it.dom()?.nodes[id].parent {
                let (l, i) = siblings(it, id);
                it.dom_insert(p, node, l.get(i + 1).copied())?;
            }
        }
        _ => return Err(it.error_ctl("SyntaxError", &format!("The value provided ('{pos}') is not one of 'beforebegin', 'afterbegin', 'beforeend', or 'afterend'."))),
    }
    Ok(())
}

pub fn content_target(it: &mut Interp, id: NodeId) -> Res<NodeId> {
    if it.tag_name(id) == "template" {
        return template_fragment(it, id);
    }
    Ok(id)
}

pub fn template_fragment(it: &mut Interp, id: NodeId) -> Res<NodeId> {
    let d = it.dom()?;
    if let Some(&f) = d.nodes[id].children.iter().find(|&&c| d.nodes[c].kind == NodeKind::Fragment) {
        return Ok(f);
    }
    let frag = d.push(NodeKind::Fragment, None);
    d.nodes[frag].parent = Some(id);
    d.nodes[id].children.push(frag);
    Ok(frag)
}

fn inner_html_get(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let target = content_target(it, id)?;
    Ok(Value::string(html::serialize_inner(it.dom()?, target)))
}

fn inner_html_set(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let s = if c.arg(0).is_nullish() { Rc::from("") } else { it.arg_str(c, 0)? };
    let target = content_target(it, id)?;
    // fragment parsing with a raw-text / RCDATA context element yields a single text node
    let tag = it.tag_name(target);
    if matches!(tag.as_str(), "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext" | "textarea" | "title") {
        let text = if matches!(tag.as_str(), "textarea" | "title") { crate::html::decode_entities(&s, false) } else { s.to_string() };
        let args = [Value::string(text)];
        let call = Call { this: c.this.clone(), args: &args, callee: c.callee, new_target: None };
        return text_content_set(it, &call);
    }
    let prev = core::mem::replace(&mut it.parsing_html_scripts, true);
    let r = it.dom_set_inner_html(target, &s);
    it.parsing_html_scripts = prev;
    r?;
    Ok(Value::Undefined)
}

fn setup_char_data(it: &mut Interp, text: &Obj, comment: &Obj) {
    for p in [text, comment] {
        it.def_accessor(
            p,
            "data",
            |it, c| {
                let id = it.this_node(c)?;
                Ok(match node_kind(it, id) {
                    NodeKind::Text(t) | NodeKind::Comment(t) => Value::string(t),
                    _ => Value::str(""),
                })
            },
            |it, c| {
                let id = it.this_node(c)?;
                let s = it.arg_str(c, 0)?;
                it.dom_set_text(id, &s)?;
                Ok(Value::Undefined)
            },
        );
        it.def_getter(p, "length", |it, c| {
            let id = it.this_node(c)?;
            Ok(Value::Num(match node_kind(it, id) {
                NodeKind::Text(t) | NodeKind::Comment(t) => u16_len(&t) as f64,
                _ => 0.0,
            }))
        });
        it.def_method(p, "appendData", 1, |it, c| {
            let id = it.this_node(c)?;
            let s = it.arg_str(c, 0)?;
            if let NodeKind::Text(t) | NodeKind::Comment(t) = node_kind(it, id) {
                it.dom_set_text(id, &format!("{t}{s}"))?;
            }
            Ok(Value::Undefined)
        });
    }
    it.def_getter(text, "wholeText", |it, c| {
        let id = it.this_node(c)?;
        Ok(match node_kind(it, id) {
            NodeKind::Text(t) => Value::string(t),
            _ => Value::str(""),
        })
    });
    it.def_method(text, "splitText", 1, |it, c| {
        let id = it.this_node(c)?;
        let off = to_integer(it.arg_num(c, 0)?).max(0.0) as usize;
        let NodeKind::Text(t) = node_kind(it, id) else { return Ok(Value::Null) };
        let head = u16_slice(&t, 0, off);
        let tail = u16_slice(&t, off, u16_len(&t));
        it.dom_set_text(id, &head)?;
        let n = it.dom()?.push(NodeKind::Text(tail), None);
        if let Some(p) = it.dom()?.nodes[id].parent {
            let (l, i) = siblings(it, id);
            it.dom_insert(p, n, l.get(i + 1).copied())?;
        }
        Ok(it.wrap(n))
    });
}

// ---------------------------------------------------------------------------
// Document
// ---------------------------------------------------------------------------

fn create_element(it: &mut Interp, tag: &str) -> Res<Value> {
    let tag = tag.to_ascii_lowercase();
    if tag.contains('-') {
        if let Some((_, ctor)) = it.custom_elements.iter().find(|(t, _)| *t == tag).cloned() {
            // run the author's constructor (its `super()` creates the element)
            return it.construct(&Value::Obj(ctor), &[], None);
        }
    }
    let doc = it.dom()?;
    let id = doc.create_element(&tag);
    Ok(it.wrap(id))
}

fn setup_document(it: &mut Interp, p: &Obj, fragment: &Obj) {
    let _ = fragment;
    it.def_getter(p, "documentElement", |it, _| {
        let d = it.dom()?;
        let h = d.nodes[d.root].children.iter().copied().find(|&c| d.nodes[c].tag() == Some("html"));
        Ok(opt_wrap(it, h))
    });
    it.def_getter(p, "head", |it, _| {
        let h = it.dom()?.head;
        Ok(it.wrap(h))
    });
    it.def_accessor(
        p,
        "body",
        |it, _| {
            let b = it.dom()?.body;
            Ok(it.wrap(b))
        },
        |it, c| {
            let nb = it.node_id(&c.arg(0))?;
            let d = it.dom()?;
            let old = d.body;
            if let Some(p) = d.nodes[old].parent {
                let (l, i) = siblings(it, old);
                it.dom_remove(old)?;
                it.dom_insert(p, nb, l.get(i + 1).copied())?;
                it.dom()?.body = nb;
            }
            Ok(Value::Undefined)
        },
    );
    it.def_accessor(
        p,
        "title",
        |it, _| {
            let d = it.dom()?;
            let t = by_tag(it, d.root, "title").first().copied();
            let s = t.and_then(|t| d.text_content(t)).unwrap_or_default();
            Ok(Value::string(s.split_whitespace().collect::<Vec<_>>().join(" ")))
        },
        |it, c| {
            let s = it.arg_str(c, 0)?;
            let d = it.dom()?;
            let t = by_tag(it, d.root, "title").first().copied();
            let t = match t {
                Some(t) => t,
                None => {
                    let t = d.create_element("title");
                    let head = d.head;
                    it.dom_insert(head, t, None)?;
                    t
                }
            };
            it.dom_set_text(t, &s)?;
            Ok(Value::Undefined)
        },
    );
    it.def_method(p, "createElement", 1, |it, c| {
        let t = it.arg_str(c, 0)?;
        if t.is_empty() || t.contains(|ch: char| ch.is_whitespace() || matches!(ch, '<' | '>' | '/' | '"' | '\'' | '=')) {
            return Err(it.error_ctl("InvalidCharacterError", "The tag name provided is not a valid name."));
        }
        create_element(it, &t)
    });
    it.def_method(p, "createElementNS", 2, |it, c| {
        let t = it.arg_str(c, 1)?;
        let t = t.rsplit(':').next().unwrap_or(&t).to_string();
        let doc = it.dom()?;
        let id = doc.push(NodeKind::Element { tag: t, attributes: Vec::new() }, None);
        Ok(it.wrap(id))
    });
    it.def_method(p, "createTextNode", 1, |it, c| {
        let s = it.arg_str(c, 0)?;
        let id = it.dom()?.push(NodeKind::Text(s.to_string()), None);
        Ok(it.wrap(id))
    });
    it.def_method(p, "createComment", 1, |it, c| {
        let s = it.arg_str(c, 0)?;
        let id = it.dom()?.push(NodeKind::Comment(s.to_string()), None);
        Ok(it.wrap(id))
    });
    it.def_method(p, "createDocumentFragment", 0, |it, _| {
        let id = it.dom()?.push(NodeKind::Fragment, None);
        Ok(it.wrap(id))
    });
    it.def_method(p, "createEvent", 1, |it, c| {
        let t = it.arg_str(c, 0)?;
        let class = match t.to_ascii_lowercase().as_str() {
            "mouseevent" | "mouseevents" => "MouseEvent",
            "keyboardevent" => "KeyboardEvent",
            "customevent" => "CustomEvent",
            "uievent" | "uievents" => "UIEvent",
            _ => "Event",
        };
        let ev = it.make_event("", class, false, false, &[]);
        it.define_data(&ev, Key::str("isTrusted"), Value::Bool(false), ALL);
        for n in ["initCustomEvent", "initMouseEvent", "initKeyboardEvent", "initUIEvent"] {
            it.def_method(&ev, n, 3, |it, c| {
                if let Value::Obj(o) = &c.this {
                    let t = it.arg_str(c, 0)?;
                    it.define_data(o, Key::str("type"), Value::Str(t), ALL);
                    it.define_data(o, Key::str("bubbles"), Value::Bool(c.arg(1).truthy()), ALL);
                    it.define_data(o, Key::str("cancelable"), Value::Bool(c.arg(2).truthy()), ALL);
                    if c.args.len() > 3 {
                        it.define_data(o, Key::str("detail"), c.arg(3), ALL);
                    }
                }
                Ok(Value::Undefined)
            });
        }
        Ok(Value::Obj(ev))
    });
    it.def_method(p, "importNode", 2, |it, c| {
        let id = it.node_id(&c.arg(0))?;
        let deep = c.arg(1).truthy();
        let src = if it.tag_name(id) == "template" { template_fragment(it, id)? } else { id };
        let copy = it.dom()?.clone_node(src, deep);
        Ok(it.wrap(copy))
    });
    it.def_method(p, "adoptNode", 1, |_, c| Ok(c.arg(0)));
    it.def_method(p, "getElementsByTagName", 1, |it, c| {
        let t = it.arg_str(c, 0)?.to_ascii_lowercase();
        let root = it.dom()?.root;
        let r = by_tag(it, root, &t);
        Ok(it.node_list(&r))
    });
    it.def_method(p, "getElementsByClassName", 1, |it, c| {
        let s = it.arg_str(c, 0)?;
        let root = it.dom()?.root;
        let r = by_class(it, root, &s);
        Ok(it.node_list(&r))
    });
    it.def_method(p, "getElementsByName", 1, |it, c| {
        let s = it.arg_str(c, 0)?;
        let root = it.dom()?.root;
        let d = it.dom()?;
        let mut out = Vec::new();
        let mut st = vec![root];
        while let Some(n) = st.pop() {
            if d.nodes[n].attribute("name") == Some(&*s) {
                out.push(n);
            }
            if t_not_template(d, n) {
                st.extend(d.nodes[n].children.iter().rev().copied());
            }
        }
        Ok(it.node_list(&out))
    });
    it.def_method(p, "elementFromPoint", 2, |_, _| Ok(Value::Null));
    it.def_method(p, "elementsFromPoint", 2, |it, _| Ok(it.new_array(Vec::new())));
    it.def_method(p, "hasFocus", 0, |_, _| Ok(Value::Bool(true)));
    it.def_method(p, "execCommand", 1, |_, _| Ok(Value::Bool(false)));
    it.def_method(p, "queryCommandSupported", 1, |_, _| Ok(Value::Bool(false)));
    it.def_method(p, "getSelection", 0, |it, _| {
        let o = it.new_object();
        it.define_data(&o, Key::str("rangeCount"), Value::Num(0.0), ALL);
        it.define_data(&o, Key::str("isCollapsed"), Value::Bool(true), ALL);
        it.def_method(&o, "toString", 0, |_, _| Ok(Value::str("")));
        for n in ["removeAllRanges", "addRange", "collapse", "selectAllChildren", "removeRange", "empty"] {
            it.def_method(&o, n, 0, |_, _| Ok(Value::Undefined));
        }
        it.def_method(&o, "getRangeAt", 1, |it, _| Ok(Value::Obj(it.new_object())));
        Ok(Value::Obj(o))
    });
    it.def_method(p, "createRange", 0, |it, _| {
        let o = it.new_object();
        for n in ["setStart", "setEnd", "setStartBefore", "setStartAfter", "setEndBefore", "setEndAfter", "selectNode", "selectNodeContents", "collapse", "deleteContents", "detach"] {
            it.def_method(&o, n, 0, |_, _| Ok(Value::Undefined));
        }
        it.def_method(&o, "getBoundingClientRect", 0, |it, _| Ok(make_rect(it, 0.0, 0.0, 0.0, 0.0)));
        it.def_method(&o, "getClientRects", 0, |it, _| Ok(it.new_array(Vec::new())));
        it.def_method(&o, "createContextualFragment", 1, |it, c| {
            let s = it.arg_str(c, 0)?;
            let frag = it.dom()?.push(NodeKind::Fragment, None);
            html::parse_fragment(it.dom()?, frag, &s);
            Ok(it.wrap(frag))
        });
        it.define_data(&o, Key::str("collapsed"), Value::Bool(true), ALL);
        Ok(Value::Obj(o))
    });
    it.def_accessor(
        p,
        "cookie",
        |it, _| Ok(Value::string(it.dom.cookie.clone())),
        |it, c| {
            let s = it.arg_str(c, 0)?;
            let pair = s.split(';').next().unwrap_or("").trim().to_string();
            let Some((name, _)) = pair.split_once('=') else { return Ok(Value::Undefined) };
            let name = name.trim().to_string();
            let expired = s.to_ascii_lowercase().contains("max-age=0") || s.to_ascii_lowercase().contains("expires=thu, 01 jan 1970");
            let mut jar: Vec<String> = it.dom.cookie.split("; ").filter(|x| !x.is_empty()).map(|x| x.to_string()).collect();
            jar.retain(|x| x.split('=').next() != Some(name.as_str()));
            if !expired {
                jar.push(pair);
            }
            it.dom.cookie = jar.join("; ");
            Ok(Value::Undefined)
        },
    );
    it.def_getter(p, "readyState", |it, _| Ok(Value::str(it.dom.ready_state)));
    for n in ["URL", "documentURI"] {
        it.def_getter(p, n, |it, _| Ok(Value::string(it.dom.base_url.clone())));
    }
    it.def_getter(p, "domain", |it, _| Ok(Value::string(Url::parse(&it.dom.base_url, None).map(|u| u.host).unwrap_or_default())));
    it.def_getter(p, "referrer", |_, _| Ok(Value::str("")));
    it.def_getter(p, "defaultView", |it, _| Ok(Value::Obj(it.global.clone())));
    it.def_getter(p, "activeElement", |it, _| {
        let f = it.dom.focused.filter(|&f| it.dom().is_ok_and(|d| d.is_connected(f)));
        let id = f.unwrap_or(it.dom()?.body);
        Ok(it.wrap(id))
    });
    it.def_getter(p, "hidden", |_, _| Ok(Value::Bool(false)));
    it.def_getter(p, "visibilityState", |_, _| Ok(Value::str("visible")));
    for n in ["characterSet", "charset", "inputEncoding"] {
        it.def_getter(p, n, |_, _| Ok(Value::str("UTF-8")));
    }
    it.def_getter(p, "contentType", |_, _| Ok(Value::str("text/html")));
    it.def_getter(p, "compatMode", |it, _| Ok(Value::str(if it.dom()?.quirks { "BackCompat" } else { "CSS1Compat" })));
    it.def_getter(p, "doctype", |it, _| {
        if it.dom()?.quirks {
            return Ok(Value::Null);
        }
        let o = it.new_object();
        it.define_data(&o, Key::str("name"), Value::str("html"), ALL);
        it.define_data(&o, Key::str("nodeType"), Value::Num(10.0), ALL);
        Ok(Value::Obj(o))
    });
    it.def_getter(p, "currentScript", |it, _| Ok(opt_wrap(it, it.current_script)));
    it.def_getter(p, "scrollingElement", |it, _| {
        let d = it.dom()?;
        let h = d.nodes[d.root].children.iter().copied().find(|&c| d.nodes[c].tag() == Some("html"));
        Ok(opt_wrap(it, h))
    });
    for (name, tag) in [("forms", "form"), ("images", "img"), ("scripts", "script"), ("embeds", "embed")] {
        let g: NativeFn = match tag {
            "form" => |it, _| collection(it, "form"),
            "img" => |it, _| collection(it, "img"),
            "script" => |it, _| collection(it, "script"),
            _ => |it, _| collection(it, "embed"),
        };
        it.def_getter(p, name, g);
    }
    it.def_getter(p, "links", |it, _| {
        let root = it.dom()?.root;
        let mut r = by_tag(it, root, "a");
        r.extend(by_tag(it, root, "area"));
        r.retain(|&n| it.attr(n, "href").is_some());
        Ok(it.node_list(&r))
    });
    it.def_getter(p, "anchors", |it, _| {
        let root = it.dom()?.root;
        let mut r = by_tag(it, root, "a");
        r.retain(|&n| it.attr(n, "name").is_some());
        Ok(it.node_list(&r))
    });
    it.def_getter(p, "styleSheets", |it, _| Ok(it.new_array(Vec::new())));
    it.def_getter(p, "fonts", |it, _| {
        let o = it.new_object();
        let pr = it.promise_resolved(Value::Undefined)?;
        it.define_data(&o, Key::str("ready"), Value::Obj(pr), ALL);
        it.define_data(&o, Key::str("status"), Value::str("loaded"), ALL);
        it.def_method(&o, "load", 1, |it, _| {
            let a = it.new_array(Vec::new());
            Ok(Value::Obj(it.promise_resolved(a)?))
        });
        it.def_method(&o, "check", 1, |_, _| Ok(Value::Bool(true)));
        for n in ["add", "delete", "clear", "forEach"] {
            it.def_method(&o, n, 0, |_, _| Ok(Value::Undefined));
        }
        it.def_method(&o, "addEventListener", 2, |_, _| Ok(Value::Undefined));
        Ok(Value::Obj(o))
    });
    it.def_accessor(p, "designMode", |_, _| Ok(Value::str("off")), |_, _| Ok(Value::Undefined));
    it.def_getter(p, "implementation", |it, _| {
        let o = it.new_object();
        it.def_method(&o, "hasFeature", 0, |_, _| Ok(Value::Bool(true)));
        it.def_method(&o, "createHTMLDocument", 1, |it, c| {
            // A detached document is approximated by a fragment holder.
            let t = if c.args.is_empty() { Rc::from("") } else { it.arg_str(c, 0)? };
            let frag = it.dom()?.push(NodeKind::Fragment, None);
            let w = it.wrap(frag);
            if let Value::Obj(o) = &w {
                let body = it.dom()?.create_element("body");
                it.dom_insert(frag, body, None)?;
                let bw = it.wrap(body);
                it.define_data(o, Key::str("body"), bw, ALL);
                it.define_data(o, Key::str("title"), Value::Str(t), ALL);
                for n in ["createElement", "createTextNode", "createComment", "createDocumentFragment"] {
                    let f = it.get_str(&Value::Obj(it.protos().document.clone()), n)?;
                    it.define_data(o, Key::str(n), f, HIDDEN);
                }
            }
            Ok(w)
        });
        Ok(Value::Obj(o))
    });
    it.def_method(p, "write", 1, |it, c| doc_write(it, c));
    it.def_method(p, "writeln", 1, |it, c| doc_write(it, c));
    it.def_method(p, "open", 0, |it, c| Ok(c.this.clone()).and_then(|v| {
        let _ = it;
        Ok(v)
    }));
    it.def_method(p, "close", 0, |_, _| Ok(Value::Undefined));
}

fn collection(it: &mut Interp, tag: &str) -> Res {
    let root = it.dom()?.root;
    let r = by_tag(it, root, tag);
    Ok(it.node_list(&r))
}

fn doc_write(it: &mut Interp, c: &Call) -> Res {
    let mut s = String::new();
    for a in c.args {
        s.push_str(&it.to_str(a)?);
    }
    let frag = it.dom()?.push(NodeKind::Fragment, None);
    let body = it.dom()?.body;
    let ctx = it.current_script.and_then(|cs| it.dom().ok().and_then(|d| d.nodes[cs].parent)).unwrap_or(body);
    html::parse_fragment(it.dom()?, frag, &s);
    let before = it.current_script.and_then(|cs| {
        let (l, i) = siblings(it, cs);
        l.get(i + 1).copied()
    });
    it.dom_insert(ctx, frag, before)?;
    Ok(Value::Undefined)
}

impl Interp {
    /// Hook for custom element `attributeChangedCallback`.
    pub fn attribute_changed(&mut self, id: NodeId, name: &str) -> Res<()> {
        let tag = self.tag_name(id);
        if !tag.contains('-') {
            return Ok(());
        }
        let Some((_, ctor)) = self.custom_elements.iter().find(|(t, _)| *t == tag).cloned() else { return Ok(()) };
        let observed = self.get_str(&Value::Obj(ctor), "observedAttributes")?;
        if observed.is_nullish() {
            return Ok(());
        }
        let list = self.array_like_to_vec(&observed)?;
        let mut hit = false;
        for v in list {
            if &*self.to_str(&v)? == name {
                hit = true;
            }
        }
        if !hit {
            return Ok(());
        }
        let w = self.wrap(id);
        let cb = self.get_str(&w, "attributeChangedCallback")?;
        if cb.as_obj().is_some_and(|o| o.is_callable()) {
            let new = self.attr(id, &name.to_ascii_lowercase()).map_or(Value::Null, Value::string);
            if let Err(Ctl::Throw(e)) = self.call(&cb, w, &[Value::str(name), Value::Null, new]) {
                self.report_error(&e);
            }
        }
        Ok(())
    }

    /// Run a custom element constructor against an existing node.
    pub fn upgrade_element(&mut self, n: NodeId, ctor: &Obj) {
        self.upgraded.insert(n);
        self.upgrade_target = Some(n);
        if let Err(Ctl::Throw(e)) = self.construct(&Value::Obj(ctor.clone()), &[], None) {
            self.report_error(&e);
        }
        self.upgrade_target = None;
        // attributeChangedCallback for attributes present at upgrade time
        let w = self.wrap(n);
        let observed = self.get_str(&Value::Obj(ctor.clone()), "observedAttributes").ok();
        let cb = self.get_str(&w, "attributeChangedCallback").ok();
        if let (Some(obs), Some(cb)) = (observed, cb) {
            if cb.as_obj().is_some_and(|o| o.is_callable()) {
                let names = self.iter_to_vec(&obs).unwrap_or_default();
                for nm in names {
                    let nm = self.to_string_lossy(&nm);
                    let val = self.doc().and_then(|d| d.nodes[n].attribute(&nm).map(String::from));
                    if let Some(v) = val {
                        if let Err(Ctl::Throw(e)) = self.call(&cb, w.clone(), &[Value::string(nm), Value::Null, Value::string(v)]) {
                            self.report_error(&e);
                        }
                    }
                }
            }
        }
    }

    /// Run `connectedCallback`/`disconnectedCallback` for custom elements in the subtree of `id`.
    pub fn custom_element_callbacks(&mut self, id: NodeId, name: &str) {
        if self.custom_elements.is_empty() {
            return;
        }
        let mut stack = vec![id];
        let mut found = Vec::new();
        while let Some(n) = stack.pop() {
            let Some(d) = self.doc() else { return };
            if d.nodes[n].tag().is_some_and(|t| t.contains('-')) {
                found.push(n);
            }
            stack.extend(d.nodes[n].children.iter().rev().copied());
            if let Some(&sr) = d.shadow_roots.get(&n) {
                stack.push(sr);
            }
        }
        for n in found {
            let tag = self.tag_name(n);
            let Some(ctor) = self.custom_elements.iter().find(|(t, _)| *t == tag).map(|(_, c)| c.clone()) else {
                continue;
            };
            // elements parsed or cloned before (or without) construction upgrade when they connect
            if name == "connectedCallback" && !self.upgraded.contains(&n) {
                self.upgrade_element(n, &ctor);
            }
            let w = self.wrap(n);
            if let Ok(cb) = self.get_str(&w, name) {
                if cb.as_obj().is_some_and(|o| o.is_callable()) {
                    if let Err(Ctl::Throw(e)) = self.call(&cb, w, &[]) {
                        self.report_error(&e);
                    }
                }
            }
        }
    }
}

fn slot_assigned(it: &mut Interp, slot: NodeId) -> Res<Vec<NodeId>> {
    let d = it.dom()?;
    if d.nodes[slot].tag() != Some("slot") {
        return Ok(Vec::new());
    }
    let name = d.nodes[slot].attribute("name").unwrap_or("").to_string();
    Ok(match crate::shadow::host_of(d, slot) {
        Some(host) => crate::shadow::assigned_nodes(d, host, &name),
        None => Vec::new(),
    })
}
