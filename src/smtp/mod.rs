//! SMTP backend abstraction.

use std::future::Future;

use lettre::{
    message::{header::ContentType, Message as LettreMessage},
    transport::smtp::{authentication::Credentials, client::{Tls, TlsParameters}, AsyncSmtpTransport},
    AsyncTransport, Tokio1Executor,
};

use crate::{config::Config, models::Message};

pub trait SmtpBackend: Send + Sync {
    fn send(&self, msg: &Message) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub struct LoopbackSmtpBackend {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub mail_from: String,
    pub mail_to: String,
}

impl LoopbackSmtpBackend {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            host: cfg.smtp_host.clone(),
            port: cfg.smtp_port,
            user: cfg.smtp_user.clone(),
            pass: cfg.smtp_pass.clone(),
            mail_from: cfg.mail_from.clone(),
            mail_to: cfg.mail_to.clone(),
        }
    }
}

impl SmtpBackend for LoopbackSmtpBackend {
    async fn send(&self, msg: &Message) -> anyhow::Result<()> {
        // Subject falls back to a sensible default so the triage inbox always has something.
        // When the submitter omitted `name`, the local-part of their email stands in so the
        // default subject still identifies the sender.
        let display_name = if msg.name.trim().is_empty() {
            msg.email.split('@').next().unwrap_or("anonymous")
        } else {
            msg.name.as_str()
        };
        let subject = if msg.subject.trim().is_empty() {
            format!("Contact form: {}", display_name)
        } else {
            msg.subject.clone()
        };

        let body = format!(
            "Received at: {}\nName: {}\nEmail: {}\nSubject: {}\nClient IP: {}\nUser-Agent: {}\n\n--- message ---\n{}\n",
            msg.received_at.to_rfc3339(),
            msg.name,
            msg.email,
            msg.subject,
            msg.client_ip.as_deref().unwrap_or("-"),
            msg.user_agent.as_deref().unwrap_or("-"),
            msg.body,
        );

        let email = LettreMessage::builder()
            .from(self.mail_from.parse()?)
            .reply_to(msg.email.parse()?)
            .to(self.mail_to.parse()?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)?;

        // The local SMTP host presents a self-signed cert on 127.0.0.1 (its
        // real cert is for the public mail hostname). We're loopback-only —
        // tolerate it explicitly rather than letting lettre refuse the
        // connection.
        let tls_params = TlsParameters::builder(self.host.clone())
            .dangerous_accept_invalid_certs(true)
            .dangerous_accept_invalid_hostnames(true)
            .build()?;

        let transport: AsyncSmtpTransport<Tokio1Executor> =
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.host)
                .port(self.port)
                .tls(Tls::Required(tls_params))
                .credentials(Credentials::new(self.user.clone(), self.pass.clone()))
                .build();

        transport.send(email).await?;
        tracing::info!(id = %msg.id, host = %self.host, "smtp send ok");
        Ok(())
    }
}
