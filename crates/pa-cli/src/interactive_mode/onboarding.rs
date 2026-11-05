use super::{PathBuf, Result, RunOptions};

#[derive(Clone)]
pub(super) struct StartupModelProbe {
    pub(super) cwd: PathBuf,
    pub(super) agent_dir: PathBuf,
    pub(super) cli_provider: Option<String>,
    pub(super) cli_model: Option<String>,
    pub(super) models: Option<Vec<String>>,
    pub(super) is_continuing: bool,
    pub(super) api_key: Option<String>,
}
impl StartupModelProbe {
    fn resolve(&self) -> (Option<pa_types::ai::Model>, bool) {
        let settings = pa_core::settings::SettingsManager::create(&self.cwd, &self.agent_dir);
        let auth = pa_core::auth::AuthStorage::create(&self.agent_dir);
        let mut registry =
            pa_core::models::ModelRegistry::create(auth, self.agent_dir.join("models.json"));
        registry.load_private_authorization_from_cache();
        let all = registry.get_all().to_vec();
        let available: Vec<_> = registry.get_available().into_iter().cloned().collect();
        let scoped = self
            .models
            .as_deref()
            .map(|p| pa_core::models::resolve_model_scope_from_models(p, &available))
            .unwrap_or_default();
        let model = pa_core::models::find_initial_model(&pa_core::models::InitialModelOptions {
            cli_provider: self.cli_provider.as_deref(),
            cli_model: self.cli_model.as_deref(),
            scoped_models: &scoped,
            is_continuing: self.is_continuing,
            default_provider: settings.get_default_provider(),
            default_model_id: settings.get_default_model(),
            all_models: &all,
            available_models: &available,
        });
        let ready = model
            .as_ref()
            .is_some_and(|m| registry.has_configured_auth(m) || self.api_key.is_some());
        (model, ready)
    }
}
pub(super) struct SettingsOnboardingSink {
    pub(super) cwd: PathBuf,
    pub(super) agent_dir: PathBuf,
}
impl pa_tui::interactive::OnboardingSink for SettingsOnboardingSink {
    fn onboarding_shown(&self) -> bool {
        pa_core::settings::SettingsManager::create(&self.cwd, &self.agent_dir)
            .get_onboarding_shown()
    }
    fn agent_traces_choice_written(&self) -> bool {
        pa_core::settings::SettingsManager::create(&self.cwd, &self.agent_dir)
            .agent_traces_choice_written()
    }
    fn set_agent_traces_enabled(&self, enabled: bool) -> Result<()> {
        pa_core::settings::SettingsManager::create(&self.cwd, &self.agent_dir)
            .set_agent_traces_enabled(enabled)
    }
    fn mark_onboarding_complete(&self) -> Result<()> {
        pa_core::settings::SettingsManager::create(&self.cwd, &self.agent_dir)
            .set_onboarding_shown(true)
    }
}
pub(super) fn onboarding_task(
    options: &RunOptions,
    provider_auth: Option<pa_tui::provider_auth::ProviderAuthCommandsHandle>,
) -> Option<pa_tui::interactive::OnboardingTask> {
    let c = &options.config;
    let s = pa_core::settings::SettingsManager::create(&c.cwd, &c.agent_dir);
    if s.get_onboarding_shown() {
        return None;
    }
    let probe = StartupModelProbe {
        cwd: c.cwd.clone(),
        agent_dir: c.agent_dir.clone(),
        cli_provider: c.provider.clone(),
        cli_model: c.model.clone(),
        models: c.models.clone(),
        is_continuing: options.session.resume.is_some()
            || options.session.continue_recent
            || options.session.fork.is_some(),
        api_key: c.api_key.clone(),
    };
    let (current_model, _) = probe.resolve();
    let ready = probe.clone();
    Some(pa_tui::interactive::OnboardingTask {
        sink: std::sync::Arc::new(SettingsOnboardingSink {
            cwd: c.cwd.clone(),
            agent_dir: c.agent_dir.clone(),
        }),
        model_ready: std::sync::Arc::new(move || ready.resolve().1),
        current_model,
        provider_auth,
    })
}
