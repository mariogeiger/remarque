mod persistence;
mod relay;
mod settings;

use relay::serve;
use settings::RelaySettings;

#[tokio::main]
async fn main() {
    let result = match RelaySettings::from_environment() {
        Ok(settings) => serve(settings).await,
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("remarque_page_relay_failed={error}");
        std::process::exit(1);
    }
}
