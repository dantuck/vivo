//! Main entry point for Vivo
use log::{debug, error, info};
use vivo::BackupConfig;
use vivo::VivoConfig;

fn main() {
    env_logger::init();
    debug!("Initializing Vivo");

    let config = VivoConfig::from_args();

    debug!("{:?}", config);

    if let Ok(backup_config) = BackupConfig::load_config(&config) {
        info!("Using default_task [{}]", backup_config.default_task);

        if let Some(task) = backup_config
            .tasks
            .iter()
            .find(|&task| task.name == backup_config.default_task)
        {
            task.run(&config, &backup_config.tasks)

            // ui::info(&format!("Backup task [{}] completed.", self.name));
        } else {
            error!(
                "default_task [{}] is not configured",
                backup_config.default_task
            );
        }
    } else {
        eprintln!("backup config not found");
    }
}
