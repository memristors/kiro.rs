//! 新增凭据时自动分配代理
//!
//! 开启后，没有自带代理设置的新凭据会按模板得到一个专属代理：地址和密码固定，
//! 用户名里的 `{id}` 替换成凭据 ID。典型用法是配合「用户名绑定出口」的代理入口，
//! 例如 `kiro-session-k{id}`：每个账号固定在自己的出口上，不同账号落到不同出口。
//!
//! 只影响新增的凭据；已有凭据不动。凭据自己填了代理（包括显式的 `direct`）时一律以它为准。

use crate::kiro::model::credentials::KiroCredentials;
use crate::model::config::Config;

/// 用户名模板里会被替换成凭据 ID 的占位符
pub const ID_PLACEHOLDER: &str = "{id}";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoProxyConfig {
    pub enabled: bool,
    pub proxy_url: Option<String>,
    pub username_template: Option<String>,
    pub password: Option<String>,
}

impl AutoProxyConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            enabled: config.auto_proxy_enabled,
            proxy_url: config.auto_proxy_url.clone(),
            username_template: config.auto_proxy_username_template.clone(),
            password: config.auto_proxy_password.clone(),
        }
    }

    /// 已开启、配置了代理地址，并且该凭据自己没有代理设置时返回 true。
    /// 凭据的 proxyUrl 为 `direct`（显式直连）也算"有设置"，不会被覆盖。
    pub fn applies_to(&self, credential: &KiroCredentials) -> bool {
        if !self.enabled {
            return false;
        }
        let has_target = self
            .proxy_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty());
        let credential_has_own = credential
            .proxy_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty());
        has_target && !credential_has_own
    }

    /// 按凭据 ID 渲染用户名；模板为空时返回 None（不带认证）
    pub fn render_username(&self, id: u64) -> Option<String> {
        self.username_template
            .as_deref()
            .map(str::trim)
            .filter(|template| !template.is_empty())
            .map(|template| template.replace(ID_PLACEHOLDER, &id.to_string()))
    }

    /// 把自动代理写进凭据的代理字段
    pub fn assign(&self, credential: &mut KiroCredentials, id: u64) {
        credential.proxy_url = self.proxy_url.as_ref().map(|url| url.trim().to_string());
        credential.proxy_username = self.render_username(id);
        credential.proxy_password = self.password.clone().filter(|p| !p.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AutoProxyConfig {
        AutoProxyConfig {
            enabled: true,
            proxy_url: Some("http://10.0.0.1:8443".to_string()),
            username_template: Some("kiro-session-k{id}".to_string()),
            password: Some("secret".to_string()),
        }
    }

    #[test]
    fn renders_id_into_username() {
        assert_eq!(config().render_username(7).as_deref(), Some("kiro-session-k7"));
    }

    #[test]
    fn template_without_placeholder_is_used_as_is() {
        let mut cfg = config();
        cfg.username_template = Some("shared".to_string());
        assert_eq!(cfg.render_username(7).as_deref(), Some("shared"));
        cfg.username_template = Some("   ".to_string());
        assert_eq!(cfg.render_username(7), None);
    }

    #[test]
    fn assigns_all_three_fields() {
        let mut cred = KiroCredentials::default();
        config().assign(&mut cred, 12);
        assert_eq!(cred.proxy_url.as_deref(), Some("http://10.0.0.1:8443"));
        assert_eq!(cred.proxy_username.as_deref(), Some("kiro-session-k12"));
        assert_eq!(cred.proxy_password.as_deref(), Some("secret"));
    }

    #[test]
    fn applies_only_to_credentials_without_their_own_proxy() {
        let cfg = config();
        assert!(cfg.applies_to(&KiroCredentials::default()));

        let mut with_proxy = KiroCredentials::default();
        with_proxy.proxy_url = Some("socks5://other:1080".to_string());
        assert!(!cfg.applies_to(&with_proxy));

        let mut direct = KiroCredentials::default();
        direct.proxy_url = Some("direct".to_string());
        assert!(!cfg.applies_to(&direct), "显式 direct 不能被覆盖");

        let mut blank = KiroCredentials::default();
        blank.proxy_url = Some("  ".to_string());
        assert!(cfg.applies_to(&blank), "空白地址视为没填");
    }

    #[test]
    fn disabled_or_missing_url_never_applies() {
        let mut cfg = config();
        cfg.enabled = false;
        assert!(!cfg.applies_to(&KiroCredentials::default()));

        let mut cfg = config();
        cfg.proxy_url = None;
        assert!(!cfg.applies_to(&KiroCredentials::default()));
        cfg.proxy_url = Some("  ".to_string());
        assert!(!cfg.applies_to(&KiroCredentials::default()));
    }

    #[test]
    fn empty_password_is_not_stored() {
        let mut cfg = config();
        cfg.password = Some(String::new());
        let mut cred = KiroCredentials::default();
        cfg.assign(&mut cred, 1);
        assert_eq!(cred.proxy_password, None);
    }
}
