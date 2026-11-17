use std::any::{Any, TypeId, type_name};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::signals::Version;

#[derive(Clone, Debug, thiserror::Error)]
pub enum StateError {
    #[error("state dependency cycle: {0}")]
    Cycle(String),
    #[error("construction input {kind} was already published")]
    AlreadyConfigured { kind: &'static str },
    #[error("missing construction input {0}")]
    MissingInput(&'static str),
    #[error("cannot construct {kind}: {detail}")]
    Construction { kind: &'static str, detail: String },
    #[error("state registry type mismatch for {0}")]
    TypeMismatch(&'static str),
}

/// Wire dependencies without committing simulated work. Each manager is
/// constructed on its first request, even when that request comes from a late task.
pub trait StateManager: Sized + 'static {
    fn create(states: &States) -> Result<Self, StateError>;
}

#[derive(Clone)]
struct Key {
    id: TypeId,
    name: &'static str,
}

impl Key {
    fn of<T: 'static>() -> Self {
        Self { id: TypeId::of::<T>(), name: type_name::<T>() }
    }
}

#[derive(Default)]
struct Registry {
    initialized: Version,
    managers: FxHashMap<TypeId, Box<dyn Any>>,
    inputs: FxHashMap<TypeId, Box<dyn Any>>,
    constructing: Vec<Key>,
}

/// One resolver per simulation run. Clones share that run's instances; factories
/// resolve dependencies synchronously, so a stack detects construction cycles.
#[derive(Clone, Default)]
pub struct States {
    registry: Rc<RefCell<Registry>>,
}

impl States {
    pub fn get<M: StateManager>(&self) -> Result<Arc<M>, StateError> {
        if let Some(manager) = self.existing::<M>() {
            return Ok(manager);
        }
        let key = Key::of::<M>();
        {
            let mut registry = self.registry.borrow_mut();
            if let Some(start) = registry.constructing.iter().position(|entry| entry.id == key.id) {
                let mut path: Vec<_> = registry.constructing[start..].iter().map(|entry| entry.name).collect();
                path.push(key.name);
                return Err(StateError::Cycle(path.join(" -> ")));
            }
            registry.constructing.push(key.clone());
        }
        let _construction = Construction(self.registry.clone());
        let manager = Arc::new(M::create(self)?);
        self.registry.borrow_mut().managers.insert(key.id, Box::new(manager.clone()));
        self.initialization_version().bump();
        Ok(manager)
    }

    /// Inspect a run without constructing equipment just to report on it.
    pub fn existing<M: StateManager>(&self) -> Option<Arc<M>> {
        self.registry.borrow().managers.get(&TypeId::of::<M>())?.downcast_ref::<Arc<M>>().cloned()
    }

    pub fn initialization_version(&self) -> Version {
        self.registry.borrow().initialized.clone()
    }

    /// Publish immutable construction inputs once. Initial inventory and other
    /// simulated setup belong in tasks that mutate their specific managers.
    pub fn configure<C: 'static>(&self, value: C) -> Result<(), StateError> {
        let mut registry = self.registry.borrow_mut();
        if registry.inputs.contains_key(&TypeId::of::<C>()) {
            return Err(StateError::AlreadyConfigured { kind: type_name::<C>() });
        }
        registry.inputs.insert(TypeId::of::<C>(), Box::new(Arc::new(value)));
        Ok(())
    }

    pub fn configuration<C: 'static>(&self) -> Result<Arc<C>, StateError> {
        self.registry
            .borrow()
            .inputs
            .get(&TypeId::of::<C>())
            .ok_or(StateError::MissingInput(type_name::<C>()))?
            .downcast_ref::<Arc<C>>()
            .cloned()
            .ok_or(StateError::TypeMismatch(type_name::<C>()))
    }
}

#[cfg(any(test, feature = "test-support"))]
impl States {
    /// Publish a manager a test built itself, so a task that resolves it
    /// from a task context reaches the test's instance instead of
    /// constructing its own.
    pub fn provide<M: StateManager>(&self, manager: Arc<M>) {
        let previous = self.registry.borrow_mut().managers.insert(TypeId::of::<M>(), Box::new(manager));
        assert!(previous.is_none(), "{} was already in place", type_name::<M>());
    }
}

struct Construction(Rc<RefCell<Registry>>);

impl Drop for Construction {
    fn drop(&mut self) {
        self.0.borrow_mut().constructing.pop();
    }
}
