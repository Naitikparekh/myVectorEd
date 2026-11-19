//! Host classes: objects that stand for something the host keeps, with properties and methods the
//! host answers ([`Realm::define_host_class`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::*;

/// The name kept for each item id, and the host calls seen.
#[derive(Default)]
struct Store {
    names: HashMap<String, String>,
    calls: Vec<String>,
}

fn id_of(d: Option<&Data>) -> Option<String> {
    match d {
        Some(Data::Host(r)) => Some(r.id.clone()),
        _ => None,
    }
}

/// A realm with the class `Item` (a read-write `name`, a read-only `id`, and `rename(text)` and
/// `same(other)` methods), the value class `Swatch` with a constructor, and `host.first`,
/// `host.all` and `host.give(value)`.
fn realm() -> (Realm, Rc<RefCell<Store>>) {
    let store: Rc<RefCell<Store>> = Rc::default();
    store.borrow_mut().names.extend([("a".to_string(), "Alpha".to_string()), ("b".to_string(), "Beta".to_string())]);
    let mut r = Realm::new();
    let get_name: HostFunction = {
        let s = store.clone();
        Rc::new(move |a| {
            let id = id_of(a.first()).ok_or_else(|| HostError::Type("not an item".into()))?;
            Ok(s.borrow().names.get(&id).cloned().map_or(Data::Undefined, Data::String))
        })
    };
    let set_name: HostFunction = {
        let s = store.clone();
        Rc::new(move |a| {
            let id = id_of(a.first()).ok_or_else(|| HostError::Type("not an item".into()))?;
            let Some(Data::String(v)) = a.get(1) else { return Err(HostError::Type("a name is text".into())) };
            s.borrow_mut().names.insert(id, v.clone());
            Ok(Data::Undefined)
        })
    };
    let get_id: HostFunction = Rc::new(|a| Ok(id_of(a.first()).map_or(Data::Undefined, Data::String)));
    let same: HostFunction = {
        let s = store.clone();
        Rc::new(move |a| {
            s.borrow_mut().calls.push(format!("same {:?} {:?}", id_of(a.first()), id_of(a.get(1))));
            Ok(Data::Bool(id_of(a.first()).is_some() && id_of(a.first()) == id_of(a.get(1))))
        })
    };
    let item = HostClass {
        name: "Item".into(),
        properties: vec![
            HostProperty { name: "name".into(), get: get_name, set: Some(set_name) },
            HostProperty { name: "id".into(), get: get_id, set: None },
        ],
        methods: vec![("same".into(), same)],
        constructor: None,
    };
    r.define_host_class(item).unwrap();
    let swatch = HostClass {
        name: "Swatch".into(),
        properties: vec![],
        methods: vec![],
        constructor: Some(Rc::new(|_| Ok(Data::Object(vec![("typename".into(), Data::String("Swatch".into())), ("red".into(), Data::Number(0.0))])))),
    };
    r.define_host_class(swatch).unwrap();
    let first: HostFunction = Rc::new(|_| Ok(Data::Host(HostRef::new("Item", "a"))));
    let all: HostFunction = Rc::new(|_| {
        let items = vec![Data::Host(HostRef::new("Item", "a")), Data::Host(HostRef::new("Item", "b"))];
        Ok(Data::Host(HostRef::list("Items", "all", items)))
    });
    let give: HostFunction = {
        let s = store.clone();
        Rc::new(move |a| {
            s.borrow_mut().calls.push(format!("give {:?}", a.first()));
            Ok(Data::Undefined)
        })
    };
    r.define_host_object(
        "host",
        vec![("first".into(), HostItem::Getter(first)), ("all".into(), HostItem::Getter(all)), ("give".into(), HostItem::Function(give))],
    )
    .unwrap();
    (r, store)
}

fn text(s: &str) -> JsValue {
    JsValue::String(s.into())
}

#[test]
fn a_host_object_reads_and_writes_its_properties_through_the_host() {
    let (mut r, store) = realm();
    assert_eq!(r.run("var i = host.first; var was = i.name; i.name = 'Renamed'; was + '/' + i.name").unwrap(), text("Alpha/Renamed"));
    assert_eq!(store.borrow().names.get("a").map(String::as_str), Some("Renamed"));
}

#[test]
fn a_read_only_property_ignores_an_assignment_and_strict_writes_throw() {
    let (mut r, _) = realm();
    assert_eq!(r.run("var i = host.first; i.id = 'z'; i.id").unwrap(), text("a"));
    assert_eq!(r.run("'use strict'; var e; try { host.first.id = 'z'; } catch (x) { e = x.name; } e").unwrap(), text("TypeError"));
}

#[test]
fn a_host_object_passed_back_reaches_the_host_as_its_reference() {
    let (mut r, store) = realm();
    assert_eq!(r.run("host.first.same(host.first)").unwrap(), JsValue::Bool(true));
    assert_eq!(r.run("host.first.same(host.all[1])").unwrap(), JsValue::Bool(false));
    r.run("host.give({inner: [host.first]})").unwrap();
    let calls = store.borrow().calls.clone();
    assert!(calls.iter().any(|c| c.contains("Host(HostRef { class: \"Item\", id: \"a\", items: None })")), "{calls:?}");
}

#[test]
fn a_method_called_on_another_value_is_given_undefined_for_the_object() {
    let (mut r, store) = realm();
    assert_eq!(r.run("Item.prototype.same.call({}, host.first)").unwrap(), JsValue::Bool(false));
    assert!(store.borrow().calls.iter().any(|c| c == "same None Some(\"a\")"));
}

#[test]
fn a_list_reference_has_its_items_and_a_length_a_script_cannot_change() {
    let (mut r, _) = realm();
    assert_eq!(r.run("var l = host.all; l.length + ':' + l[0].name + ',' + l[1].name").unwrap(), text("2:Alpha,Beta"));
    assert_eq!(r.run("var l = host.all; l.length = 9; l[0] = 1; l.length + typeof l[0]").unwrap(), text("2object"));
}

#[test]
fn the_hidden_reference_cannot_be_changed_or_listed() {
    let (mut r, _) = realm();
    assert_eq!(r.run("var i = host.first; i.__vcHostId = 'b'; delete i.__vcHostId; i.name").unwrap(), text("Alpha"));
    assert_eq!(r.run("var k = []; for (var p in host.first) { k.push(p); } k.join(',')").unwrap(), text(""));
    assert_eq!(r.run("JSON.stringify(host.first)").unwrap(), text("{}"));
}

#[test]
fn objects_are_instances_of_their_class() {
    let (mut r, _) = realm();
    assert_eq!(
        r.run("host.first instanceof Item && host.first.constructor === Item && !(host.first instanceof Swatch)").unwrap(),
        JsValue::Bool(true)
    );
}

#[test]
fn a_class_with_a_constructor_makes_plain_values_with_its_prototype() {
    let (mut r, _) = realm();
    assert_eq!(r.run("var s = new Swatch(); s.red = 5; (s instanceof Swatch) + ':' + s.typename + ':' + s.red").unwrap(), text("true:Swatch:5"));
    assert_eq!(r.run("Swatch() instanceof Swatch").unwrap(), JsValue::Bool(true));
    assert_eq!(r.run("JSON.stringify(new Swatch())").unwrap(), text("{\"typename\":\"Swatch\",\"red\":0}"));
}

#[test]
fn a_class_without_a_constructor_cannot_be_made() {
    let (mut r, _) = realm();
    assert_eq!(
        r.run("var e; try { new Item(); } catch (x) { e = x.name + ': ' + x.message; } e").unwrap(),
        text("TypeError: Item cannot be made by a script")
    );
    assert_eq!(r.run("var e; try { Item(); } catch (x) { e = x.name; } e").unwrap(), text("TypeError"));
}

#[test]
fn a_reference_to_a_class_never_defined_still_crosses_back() {
    let mut r = Realm::new();
    let seen: Rc<RefCell<Option<Data>>> = Rc::default();
    let s = seen.clone();
    let give: HostFunction = Rc::new(|_| Ok(Data::Host(HostRef::new("Ghost", "g1"))));
    let take: HostFunction = Rc::new(move |a| {
        *s.borrow_mut() = a.first().cloned();
        Ok(Data::Undefined)
    });
    r.define_host_object("host", vec![("give".into(), HostItem::Function(give)), ("take".into(), HostItem::Function(take))]).unwrap();
    r.run("host.take(host.give())").unwrap();
    assert_eq!(*seen.borrow(), Some(Data::Host(HostRef::new("Ghost", "g1"))));
}

#[test]
fn class_prototypes_survive_a_collection() {
    let (mut r, _) = realm();
    r.run("Item = null; Swatch = null;").unwrap();
    r.collect_garbage();
    assert_eq!(r.run("var i = host.first; typeof i.same + ':' + i.name").unwrap(), text("function:Alpha"));
}
