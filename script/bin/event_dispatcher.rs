use async_trait::async_trait;
use anyhow::Result;
use sp1_helios_script::event_dispatcher::{EventDispatcher, EventListener};

pub struct PrintListener1;

#[async_trait]
impl EventListener<String> for PrintListener1 {
    async fn on_event(&mut self, data: String) -> Result<()> {
        println!("PrintListener1 received data: {}", data);
        Ok(())
    }
}


pub struct PrintListener2;

#[async_trait]
impl EventListener<String> for PrintListener2 {
    async fn on_event(&mut self, data: String) -> Result<()> {
        println!("PrintListener2 received data: {}", data);
        Ok(())
    }
}


#[tokio::main]
async fn main() -> Result<()> {
    let mut dispatcher = EventDispatcher::<String>::new();

    let listener1 = PrintListener1;
    let listener2 = PrintListener2;

    dispatcher.add_listener(listener1);
    dispatcher.add_listener(listener2);

    dispatcher.trigger("1".to_string()).await?;
    dispatcher.trigger("2".to_string()).await?;
    dispatcher.trigger("3".to_string()).await?;
    dispatcher.trigger("4".to_string()).await?;

    Ok(())
}