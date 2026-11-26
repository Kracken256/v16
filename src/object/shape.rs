use crate::runtime::atom::Atom;
use crate::util::FxHashMap;
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::{Rc, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShapeId(pub usize);

impl ShapeId {
    pub fn root() -> Self {
        ShapeId(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Transition {
    pub property: Atom,
    pub offset: u32,
}

pub struct Shape {
    pub id: ShapeId,
    pub parent: Option<Rc<Shape>>,
    pub transition: Option<Transition>,
    pub property_count: u32,
    property_map: OnceCell<FxHashMap<Atom, u32>>,
    children: RefCell<Vec<(Atom, Weak<Shape>)>>,
}

impl Shape {
    pub fn root() -> Self {
        Shape {
            id: ShapeId::root(),
            parent: None,
            transition: None,
            property_count: 0,
            property_map: OnceCell::new(),
            children: RefCell::new(Vec::new()),
        }
    }

    pub fn add_property(self: &Rc<Self>, property: Atom, id: ShapeId) -> Rc<Shape> {
        let offset = self.property_count;
        Rc::new(Shape {
            id,
            parent: Some(Rc::clone(self)),
            transition: Some(Transition { property, offset }),
            property_count: self.property_count + 1,
            property_map: OnceCell::new(),
            children: RefCell::new(Vec::new()),
        })
    }

    pub fn get_offset(&self, property: Atom) -> Option<u32> {
        if self.property_count > 8 {
            return self
                .property_map()
                .and_then(|map| map.get(&property).copied());
        }

        if let Some(transition) = self.transition {
            if transition.property == property {
                return Some(transition.offset);
            }
        }
        self.parent
            .as_ref()
            .and_then(|parent| parent.get_offset(property))
    }

    fn property_map(&self) -> Option<&FxHashMap<Atom, u32>> {
        self.property_map.get_or_init(|| {
            let mut map = FxHashMap::default();
            map.reserve(self.property_count as usize);
            if let Some(transition) = self.transition {
                map.insert(transition.property, transition.offset);
            }
            let mut current = self.parent.as_deref();
            while let Some(shape) = current {
                if let Some(transition) = shape.transition {
                    map.insert(transition.property, transition.offset);
                }
                current = shape.parent.as_deref();
            }
            map
        });
        self.property_map.get()
    }

    pub fn properties(&self) -> Vec<(Atom, u32)> {
        let mut props = Vec::with_capacity(self.property_count as usize);
        if let Some(transition) = self.transition {
            props.push((transition.property, transition.offset));
        }
        let mut current = self.parent.as_deref();
        while let Some(shape) = current {
            if let Some(transition) = shape.transition {
                props.push((transition.property, transition.offset));
            }
            current = shape.parent.as_deref();
        }
        props.reverse();
        props
    }

    pub fn has_property(&self, property: Atom) -> bool {
        self.get_offset(property).is_some()
    }

    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }
}

impl std::fmt::Debug for Shape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shape")
            .field("id", &self.id)
            .field("property_count", &self.property_count)
            .field("transition", &self.transition)
            .finish()
    }
}

pub struct ShapeCache {
    root_shape: Rc<Shape>,
    next_id: Cell<usize>,
}

impl ShapeCache {
    pub fn new() -> Self {
        ShapeCache {
            root_shape: Rc::new(Shape::root()),
            next_id: Cell::new(1),
        }
    }

    pub fn alloc_id(&self) -> ShapeId {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        ShapeId(id)
    }

    pub fn root_shape(&self) -> Rc<Shape> {
        Rc::clone(&self.root_shape)
    }

    pub fn transition(&mut self, parent: &Rc<Shape>, property: Atom) -> Rc<Shape> {
        let mut children = parent.children.borrow_mut();
        children.retain(|(_, child)| child.strong_count() > 0);
        if let Some(child) = children.iter().find_map(|(atom, child)| {
            if *atom == property {
                child.upgrade()
            } else {
                None
            }
        }) {
            return child;
        }

        let child_id = self.alloc_id();
        let child = parent.add_property(property, child_id);
        children.push((property, Rc::downgrade(&child)));
        child
    }

    pub fn create_shape_for_properties(&mut self, properties: &[Atom]) -> Rc<Shape> {
        let mut current = self.root_shape();

        for property in properties {
            current = self.transition(&current, *property);
        }

        current
    }
}

impl Default for ShapeCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::atom::AtomTable;

    #[test]
    fn test_root_shape() {
        let shape = Shape::root();
        assert!(shape.is_root());
        assert_eq!(shape.property_count, 0);
    }

    #[test]
    fn test_shape_transitions() {
        let mut atoms = AtomTable::new();
        let foo = atoms.intern("foo");
        let bar = atoms.intern("bar");

        let mut cache = ShapeCache::new();
        let root = cache.root_shape();

        let shape1 = cache.transition(&root, foo);
        let s1 = &*shape1;
        assert_eq!(s1.property_count, 1);
        assert_eq!(s1.get_offset(foo), Some(0));
        assert_eq!(s1.get_offset(bar), None);

        let shape2 = cache.transition(&shape1, bar);
        let s2 = &*shape2;
        assert_eq!(s2.property_count, 2);
        assert_eq!(s2.get_offset(foo), Some(0));
        assert_eq!(s2.get_offset(bar), Some(1));
    }

    #[test]
    fn test_shape_caching() {
        let mut atoms = AtomTable::new();
        let foo = atoms.intern("foo");

        let mut cache = ShapeCache::new();
        let root = cache.root_shape();

        let shape1 = cache.transition(&root, foo);
        let shape2 = cache.transition(&root, foo);

        assert!(Rc::ptr_eq(&shape1, &shape2));
    }

    #[test]
    fn test_properties_iteration() {
        let mut atoms = AtomTable::new();
        let foo = atoms.intern("foo");
        let bar = atoms.intern("bar");
        let baz = atoms.intern("baz");

        let mut cache = ShapeCache::new();
        let root = cache.root_shape();

        let shape1 = cache.transition(&root, foo);
        let shape2 = cache.transition(&shape1, bar);
        let shape3 = cache.transition(&shape2, baz);

        let props = shape3.properties();
        assert_eq!(props.len(), 3);
        assert_eq!(props[0], (foo, 0));
        assert_eq!(props[1], (bar, 1));
        assert_eq!(props[2], (baz, 2));
    }

    #[test]
    fn test_large_shape_property_map() {
        let mut atoms = AtomTable::new();
        let mut cache = ShapeCache::new();
        let mut shape = cache.root_shape();
        let mut names = Vec::new();

        for i in 0..12 {
            let atom = atoms.intern(&format!("property_{i}"));
            names.push(atom);
            shape = cache.transition(&shape, atom);
        }

        for (offset, atom) in names.iter().enumerate() {
            assert_eq!(shape.get_offset(*atom), Some(offset as u32));
        }
        assert_eq!(shape.properties().len(), 12);
    }
}