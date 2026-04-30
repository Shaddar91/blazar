//! SMTP backend abstraction.

use std::future::Future;

use anyhow::Context;
use chrono::Datelike;
use lettre::{
    message::{header::ContentType, Mailbox, Message as LettreMessage, MultiPart},
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

    pub auto_reply_enabled: bool,
    pub auto_reply_from: String,
    pub auto_reply_html: String,
    pub auto_reply_text: String,
}

impl LoopbackSmtpBackend {
    pub fn from_config(cfg: &Config) -> anyhow::Result<Self> {
        let (auto_reply_html, auto_reply_text) = if cfg.auto_reply_enabled {
            let html = std::fs::read_to_string(&cfg.auto_reply_html_path).with_context(|| {
                format!(
                    "failed to read AUTO_REPLY_HTML_PATH at {}",
                    cfg.auto_reply_html_path
                )
            })?;
            let text = std::fs::read_to_string(&cfg.auto_reply_text_path).with_context(|| {
                format!(
                    "failed to read AUTO_REPLY_TEXT_PATH at {}",
                    cfg.auto_reply_text_path
                )
            })?;
            (html, text)
        } else {
            (String::new(), String::new())
        };

        Ok(Self {
            host: cfg.smtp_host.clone(),
            port: cfg.smtp_port,
            user: cfg.smtp_user.clone(),
            pass: cfg.smtp_pass.clone(),
            mail_from: cfg.mail_from.clone(),
            mail_to: cfg.mail_to.clone(),
            auto_reply_enabled: cfg.auto_reply_enabled,
            auto_reply_from: cfg.auto_reply_from.clone(),
            auto_reply_html,
            auto_reply_text,
        })
    }

    fn build_transport(&self) -> anyhow::Result<AsyncSmtpTransport<Tokio1Executor>> {
        //The local SMTP host presents a self-signed cert on 127.0.0.1 (its
        //real cert is for the public mail hostname). Loopback-only — tolerate
        //it explicitly rather than letting lettre refuse the connection.
        let tls_params = TlsParameters::builder(self.host.clone())
            .dangerous_accept_invalid_certs(true)
            .dangerous_accept_invalid_hostnames(true)
            .build()?;

        Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.host)
            .port(self.port)
            .tls(Tls::Required(tls_params))
            .credentials(Credentials::new(self.user.clone(), self.pass.clone()))
            .build())
    }

    pub async fn send_auto_reply(
        &self,
        to: &str,
        name: &str,
        topics: &[String],
        message_preview: &str,
        inquiry_id: &str,
    ) -> anyhow::Result<()> {
        if !self.auto_reply_enabled {
            return Ok(());
        }

        let display_name = if name.trim().is_empty() { "there" } else { name.trim() };
        let topics_joined = topics.join(", ");
        let topics_html = render_topics_html(topics);
        let message_trimmed = message_preview.trim();
        let message_text = message_trimmed.to_string();
        let message_html = html_escape(message_trimmed).replace('\n', "<br />");
        let current_year = chrono::Utc::now().year().to_string();
        let site_url = "https://cloud-lord.com";
        let contact_email = "engineering@cloud-lord.com";
        let reply_eta = "within 24 hours";

        let topics_present = !topics.is_empty();
        let message_present = !message_trimmed.is_empty();
        let is_present = |var: &str| -> bool {
            match var {
                "topics" => topics_present,
                "message_preview" => message_present,
                _ => true,
            }
        };

        let html_with_blocks = strip_conditional_blocks(&self.auto_reply_html, &is_present);
        let html = html_with_blocks
            .replace("{{name}}", display_name)
            .replace("{{topics_html}}", &topics_html)
            .replace("{{topics}}", &topics_joined)
            .replace("{{message_preview}}", &message_html)
            .replace("{{reply_eta}}", reply_eta)
            .replace("{{inquiry_id}}", inquiry_id)
            .replace("{{site_url}}", site_url)
            .replace("{{contact_email}}", contact_email)
            .replace("{{current_year}}", &current_year);

        let text_with_blocks = strip_conditional_blocks(&self.auto_reply_text, &is_present);
        let text = text_with_blocks
            .replace("{{name}}", display_name)
            .replace("{{topics}}", &topics_joined)
            .replace("{{message_preview}}", &message_text)
            .replace("{{reply_eta}}", reply_eta)
            .replace("{{inquiry_id}}", inquiry_id)
            .replace("{{site_url}}", site_url)
            .replace("{{contact_email}}", contact_email)
            .replace("{{current_year}}", &current_year);

        let from: Mailbox = self
            .auto_reply_from
            .parse()
            .with_context(|| format!("invalid AUTO_REPLY_FROM: {}", self.auto_reply_from))?;
        let to_box: Mailbox = to
            .parse()
            .with_context(|| format!("invalid auto-reply recipient: {to}"))?;

        let email = LettreMessage::builder()
            .from(from)
            .to(to_box)
            .subject("Thanks for reaching out — cloud-lord")
            .multipart(MultiPart::alternative_plain_html(text, html))?;

        let transport = self.build_transport()?;
        transport.send(email).await?;
        tracing::info!(inquiry_id = %inquiry_id, host = %self.host, "auto-reply send ok");
        Ok(())
    }
}

impl SmtpBackend for LoopbackSmtpBackend {
    async fn send(&self, msg: &Message) -> anyhow::Result<()> {
        //Subject falls back to a sensible default so the triage inbox always has something.
        //When the submitter omitted `name`, the local-part of their email stands in so the
        //default subject still identifies the sender.
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

        let transport = self.build_transport()?;
        transport.send(email).await?;
        tracing::info!(id = %msg.id, host = %self.host, "smtp send ok");
        Ok(())
    }
}

fn render_topics_html(topics: &[String]) -> String {
    if topics.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for t in topics {
        let escaped = html_escape(t);
        out.push_str(&format!(
            "<table class=\"chip\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tr><td>{}</td></tr></table>",
            escaped
        ));
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

//Minimal Handlebars `{{#if VAR}}...{{/if}}` block handler shared by html and
//text templates. For each block, `is_present(VAR)` decides: true → strip just
//the marker lines and keep the inner content; false → drop markers AND their
//content. Unknown variables (no map entry) should default to `true` in the
//caller's closure so unrelated blocks pass through untouched. Nested blocks
//are not supported — templates don't need them.
fn strip_conditional_blocks(input: &str, is_present: &dyn Fn(&str) -> bool) -> String {
    let open_prefix = "{{#if ";
    let close = "{{/if}}";
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(open_prefix) {
        out.push_str(&rest[..start]);
        let after_prefix = &rest[start + open_prefix.len()..];
        let Some(name_end) = after_prefix.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let var = after_prefix[..name_end].trim();
        let after_open = &after_prefix[name_end + 2..];
        let Some(end) = after_open.find(close) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after_open[..end];
        let after_close = &after_open[end + close.len()..];
        if is_present(var) {
            //Trim a single leading/trailing newline introduced by the marker lines
            //so the kept content sits flush against the surrounding text.
            let inner = inner.strip_prefix('\n').unwrap_or(inner);
            let inner = inner.strip_suffix('\n').unwrap_or(inner);
            out.push_str(inner);
            rest = after_close;
        } else {
            //Eat one trailing newline after `{{/if}}` so dropping the block doesn't leave a blank line.
            rest = after_close.strip_prefix('\n').unwrap_or(after_close);
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_html_empty_for_empty_slice() {
        assert_eq!(render_topics_html(&[]), "");
    }

    #[test]
    fn topics_html_renders_one_chip_per_topic() {
        let out = render_topics_html(&["rust".to_string(), "infra".to_string()]);
        assert_eq!(out.matches("<table class=\"chip\"").count(), 2);
        assert!(out.contains(">rust<"));
        assert!(out.contains(">infra<"));
    }

    #[test]
    fn topics_html_escapes_user_input() {
        let out = render_topics_html(&["<script>".to_string()]);
        assert!(out.contains("&lt;script&gt;"));
        assert!(!out.contains("<script>"));
    }

    #[test]
    fn handlebars_if_keeps_inner_when_present() {
        let tpl = "before\n{{#if topics}}\nTopics: {{topics}}\n{{/if}}\nafter";
        let out = strip_conditional_blocks(tpl, &|v| v == "topics");
        assert_eq!(out, "before\nTopics: {{topics}}\nafter");
    }

    #[test]
    fn handlebars_if_drops_block_when_absent() {
        let tpl = "before\n{{#if topics}}\nTopics: {{topics}}\n{{/if}}\nafter";
        let out = strip_conditional_blocks(tpl, &|_| false);
        assert_eq!(out, "before\nafter");
    }

    #[test]
    fn handlebars_if_unrelated_block_untouched() {
        let tpl = "{{#if other}}keep me{{/if}}";
        //Default-true for any var the caller doesn't recognize: the block stays.
        let out = strip_conditional_blocks(tpl, &|v| v != "topics");
        assert_eq!(out, "keep me");
    }

    #[test]
    fn handlebars_if_handles_multiple_distinct_vars() {
        let tpl = "A {{#if topics}}T{{/if}} B {{#if message_preview}}M{{/if}} C";
        let out = strip_conditional_blocks(tpl, &|v| match v {
            "topics" => true,
            "message_preview" => false,
            _ => true,
        });
        assert_eq!(out, "A T B  C");
    }

    #[test]
    fn handlebars_if_drops_html_block_when_absent() {
        //Mirror the shape of the actual HTML template — a multi-line <tr> wrapped
        //by the markers — and confirm the dropped block leaves no orphaned tags.
        let tpl = "<table>\n  {{#if message_preview}}\n  <tr><td>{{message_preview}}</td></tr>\n  {{/if}}\n</table>";
        let out = strip_conditional_blocks(tpl, &|_| false);
        assert!(!out.contains("<tr>"));
        assert!(!out.contains("{{message_preview}}"));
        assert!(!out.contains("{{#if"));
        assert!(!out.contains("{{/if}}"));
    }
}
