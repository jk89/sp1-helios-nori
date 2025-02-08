pub trait EventListener<T: std::clone::Clone> {
    fn on_event(&mut self, data: T);
}

pub struct EventDispatcher<T: std::clone::Clone> {
    listeners: Vec<Box<dyn EventListener<T>>>
}

impl<T: std::clone::Clone> EventDispatcher<T> {
    pub fn new() -> Self {
        Self {
            listeners: Vec::new(),
        }
    }

    pub fn add_listener<L>(&mut self, listener: L) 
    where 
        L: EventListener<T> + 'static,
    {
        self.listeners.push(Box::new(listener));
    }

    pub fn trigger(&mut self, data: T) {
        for listener in self.listeners.iter_mut() {
            listener.on_event(data.clone());
        }
    }
}

