// Quick test to verify registry date fetching
use dependi_lsp::registries::{
    Registry, crates_io::CratesIoRegistry, npm::NpmRegistry, pypi::PyPiRegistry,
};

#[tokio::main]
async fn main() {
    println!("=== Testing crates.io (serde) ===");
    let registry = CratesIoRegistry::new().unwrap();
    match registry.get_version_info("serde").await {
        Ok(info) => {
            println!(
                "Versions: {:?}",
                info.versions.iter().take(3).collect::<Vec<_>>()
            );
            println!("Release dates count: {}", info.release_dates.len());
            println!(
                "Sample dates: {:?}",
                info.release_dates.iter().take(3).collect::<Vec<_>>()
            );
        }
        Err(e) => println!("Error: {}", e),
    }

    println!("\n=== Testing npm (express) ===");
    let registry = NpmRegistry::new().unwrap();
    match registry.get_version_info("express").await {
        Ok(info) => {
            println!(
                "Versions: {:?}",
                info.versions.iter().take(3).collect::<Vec<_>>()
            );
            println!("Release dates count: {}", info.release_dates.len());
            println!(
                "Sample dates: {:?}",
                info.release_dates.iter().take(3).collect::<Vec<_>>()
            );
        }
        Err(e) => println!("Error: {}", e),
    }

    println!("\n=== Testing PyPI (flask) ===");
    let registry = PyPiRegistry::new().unwrap();
    match registry.get_version_info("flask").await {
        Ok(info) => {
            println!(
                "Versions: {:?}",
                info.versions.iter().take(3).collect::<Vec<_>>()
            );
            println!("Release dates count: {}", info.release_dates.len());
            println!(
                "Sample dates: {:?}",
                info.release_dates.iter().take(3).collect::<Vec<_>>()
            );
        }
        Err(e) => println!("Error: {}", e),
    }
}
