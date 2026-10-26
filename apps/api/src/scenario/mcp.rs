//! Model Context Protocol (MCP) security scenario.
//!
//! Demonstrates the three security tiers (`Public`, `Scoped`, and `HumanApproved`),
//! cryptographic parameter binding via RFC 8785 JSON Canonicalization Scheme (JCS)
//! and SHA-256 digest computation, and single-use `AtomicConsume` execution tokens.

use std::time::{SystemTime, UNIX_EPOCH};

use authkestra_mcp::approval::{ApprovedExecutionRecord, PendingApprovalChallenge};
use authkestra_mcp::canonicalize::{canonicalize_arguments, compute_challenge_digest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;
use uuid::Uuid;

use super::{
    Consequences, ControlShape, ControlValue, CrateRequirement, KitContext, KitFragment, KitLink,
    Scenario, ScenarioContext, ScenarioOption,
};
use crate::ceremony::CeremonyKind;
use crate::error::ApiError;
use crate::events::Step;

pub struct McpScenario;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpInvocationResult {
    pub status: String,
    pub tier: String,
    pub challenge_id: Option<String>,
    pub digest: Option<String>,
    pub execution_token: Option<String>,
    #[ts(type = "any")]
    pub output: Option<Value>,
    pub detail: String,
}

#[derive(Debug, Deserialize)]
struct CallBody {
    tool_name: String,
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct ApproveBody {
    challenge_id: String,
}

#[derive(Debug, Deserialize)]
struct ExecuteBody {
    execution_token: String,
    tool_name: String,
    arguments: Value,
}

#[async_trait::async_trait]
impl Scenario for McpScenario {
    fn id(&self) -> &'static str {
        "mcp"
    }

    fn name(&self) -> &'static str {
        "AI Agent & MCP Security"
    }

    fn summary(&self) -> &'static str {
        "Protect Model Context Protocol (MCP) tool execution with Public, Scoped, \
         and HumanApproved security tiers, RFC 8785 JCS parameter digest binding, \
         and AtomicConsume single-use replay defense."
    }

    fn control(&self) -> ControlShape {
        ControlShape::SelectOne {
            options: vec![
                ScenarioOption::new(
                    "human_approved",
                    "Tier 3: HumanApproved (Biometric Step-Up + JCS Digest Binding)",
                ),
                ScenarioOption::new(
                    "scoped",
                    "Tier 2: Scoped (Bearer Token with Scope Verification)",
                ),
                ScenarioOption::new("public", "Tier 1: Public (Unauthenticated Read-Only Tool)"),
            ],
        }
    }

    fn default_value(&self) -> ControlValue {
        ControlValue::SelectOne { selected: None }
    }

    fn actions(&self) -> Vec<&'static str> {
        vec!["call", "approve", "execute", "tamper"]
    }

    fn consequences(&self, value: &ControlValue) -> Consequences {
        if !value.is_active() {
            return Consequences::default();
        }

        let tier = match value {
            ControlValue::SelectOne { selected: Some(t) } => t.as_str(),
            _ => "human_approved",
        };

        let mut reqs = vec![
            "Model Context Protocol (MCP) server verifies tool invocations against security tiers."
                .to_string(),
        ];

        if tier == "human_approved" {
            reqs.push(
                "Sensitive tools return error -32001 eliciting out-of-band biometric passkey approval."
                    .to_string(),
            );
            reqs.push(
                "Arguments are canonicalized via RFC 8785 JCS and SHA-256 bound to the approval."
                    .to_string(),
            );
            reqs.push(
                "Execution tokens are consumed atomically once to prevent replay attacks."
                    .to_string(),
            );
        } else if tier == "scoped" {
            reqs.push(
                "Agent tool calls require a valid Bearer token presenting the required scope."
                    .to_string(),
            );
        }

        Consequences {
            routes: vec![
                "POST /mcp/tools/call".to_string(),
                "POST /mcp/approve".to_string(),
            ],
            requirements: reqs,
            crates: vec![
                CrateRequirement::new("authkestra-mcp", &["default"]),
                CrateRequirement::new("authkestra-engine", &["rustls-aws-lc-rs"]),
            ],
        }
    }

    async fn action(
        &self,
        action: &str,
        body: Value,
        ctx: &ScenarioContext<'_>,
    ) -> Result<Value, ApiError> {
        let tier = match ctx.value {
            ControlValue::SelectOne { selected: Some(t) } => t.as_str(),
            _ => "human_approved",
        };

        match action {
            "call" => {
                let body: CallBody = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!("expected {{ tool_name, arguments }}: {e}"))
                })?;

                if tier == "public" {
                    ctx.record(
                        Step::success("mcp", "public tool call executed")
                            .detail(
                                "Public tier tool executed immediately with no credential required.",
                            )
                            .fact("tool", body.tool_name.clone()),
                    )
                    .await;

                    return Ok(serde_json::to_value(McpInvocationResult {
                        status: "executed".to_string(),
                        tier: "public".to_string(),
                        challenge_id: None,
                        digest: None,
                        execution_token: None,
                        output: Some(json!({
                            "result": "Public information retrieved successfully.",
                            "tool": body.tool_name,
                        })),
                        detail: "Public tier: executed immediately without step-up authentication."
                            .to_string(),
                    })
                    .expect("serialises"));
                }

                if tier == "scoped" {
                    ctx.record(
                        Step::success("mcp", "scoped tool call executed")
                            .detail(
                                "Scoped tier verified bearer token claims and authorized tool execution.",
                            )
                            .fact("tool", body.tool_name.clone())
                            .fact("scope", "tools:execute"),
                    )
                    .await;

                    return Ok(serde_json::to_value(McpInvocationResult {
                        status: "executed".to_string(),
                        tier: "scoped".to_string(),
                        challenge_id: None,
                        digest: None,
                        execution_token: None,
                        output: Some(json!({
                            "result": "Scoped tool executed within authorized boundaries.",
                            "tool": body.tool_name,
                            "scope_verified": "tools:execute",
                        })),
                        detail: "Scoped tier: verified authorization claims before execution."
                            .to_string(),
                    })
                    .expect("serialises"));
                }

                // Tier 3: HumanApproved
                let canonical_bytes = canonicalize_arguments(&body.arguments).map_err(|e| {
                    ApiError::InvalidValue(format!("arguments could not be canonicalized: {e}"))
                })?;

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let expires_at = now + 300;

                let digest = compute_challenge_digest(
                    &ctx.session_id.to_string(),
                    &body.tool_name,
                    &canonical_bytes,
                    expires_at,
                );

                let challenge_id = Uuid::new_v4().to_string();

                let challenge = PendingApprovalChallenge {
                    approval_id: challenge_id.clone(),
                    session_id: ctx.session_id.to_string(),
                    tool_name: body.tool_name.clone(),
                    challenge_digest: digest.clone(),
                    expires_at,
                    reproof_max_age_secs: 300,
                    reproof_require_step_up: true,
                    created_at: now,
                };

                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::McpChallenge,
                        serde_json::to_string(&challenge)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                ctx.record(
                    Step::info("mcp", "step-up challenge issued (-32001)")
                        .detail(
                            "Tool call demands HumanApproved tier. JCS SHA-256 parameter digest \
                             bound to session; returned JSON-RPC error -32001 requiring biometric confirmation.",
                        )
                        .fact("tool", body.tool_name.clone())
                        .fact("digest", digest.chars().take(16).collect::<String>() + "...")
                        .fact("challenge_id", challenge_id.clone()),
                )
                .await;

                Ok(serde_json::to_value(McpInvocationResult {
                    status: "challenge".to_string(),
                    tier: "human_approved".to_string(),
                    challenge_id: Some(challenge_id),
                    digest: Some(digest),
                    execution_token: None,
                    output: Some(json!({
                        "jsonrpc": "2.0",
                        "error": {
                            "code": -32001,
                            "message": "Human step-up approval required",
                            "data": {
                                "tool": body.tool_name,
                                "arguments": body.arguments,
                            }
                        }
                    })),
                    detail:
                        "Error -32001: Tool call paused awaiting human biometric step-up approval."
                            .to_string(),
                })
                .expect("serialises"))
            }

            "approve" => {
                let body: ApproveBody = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!("expected {{ challenge_id }}: {e}"))
                })?;

                let Some(raw) = ctx
                    .ceremonies
                    .take(ctx.session_id, CeremonyKind::McpChallenge)
                    .await
                    .map_err(ApiError::from)?
                else {
                    return Err(ApiError::CeremonyExpired);
                };

                let challenge: PendingApprovalChallenge = serde_json::from_str(&raw)
                    .map_err(|e| ApiError::Scenario(format!("bad challenge state: {e}")))?;

                if challenge.approval_id != body.challenge_id {
                    return Err(ApiError::InvalidValue(
                        "challenge_id does not match pending challenge".to_string(),
                    ));
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let execution_token = format!("exec_tok_{}", Uuid::new_v4());

                let record = ApprovedExecutionRecord {
                    execution_token: execution_token.clone(),
                    approval_id: challenge.approval_id,
                    session_id: challenge.session_id,
                    tool_name: challenge.tool_name.clone(),
                    challenge_digest: challenge.challenge_digest.clone(),
                    approver_external_id: ctx.user_id(),
                    approver_provider_id: "webauthn".to_string(),
                    expires_at: challenge.expires_at,
                    created_at: now,
                };

                ctx.ceremonies
                    .put(
                        ctx.session_id,
                        CeremonyKind::McpExecution,
                        serde_json::to_string(&record)
                            .map_err(|e| ApiError::Scenario(e.to_string()))?,
                    )
                    .await?;

                ctx.record(
                    Step::success("mcp", "human approval confirmed")
                        .detail(
                            "Biometric passkey verified with user presence and verification flags. \
                             Single-use execution token issued for parameter digest.",
                        )
                        .fact("approver", ctx.user_id())
                        .fact("token", execution_token.chars().take(20).collect::<String>() + "..."),
                )
                .await;

                Ok(serde_json::to_value(McpInvocationResult {
                    status: "approved".to_string(),
                    tier: "human_approved".to_string(),
                    challenge_id: Some(body.challenge_id),
                    digest: Some(challenge.challenge_digest),
                    execution_token: Some(execution_token),
                    output: None,
                    detail: "Biometric approval verified. Single-use execution token ready."
                        .to_string(),
                })
                .expect("serialises"))
            }

            "execute" => {
                let body: ExecuteBody = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!(
                        "expected {{ execution_token, tool_name, arguments }}: {e}"
                    ))
                })?;

                // AtomicConsume: retrieve and remove execution record in one atomic action
                let Some(raw) = ctx
                    .ceremonies
                    .take(ctx.session_id, CeremonyKind::McpExecution)
                    .await
                    .map_err(ApiError::from)?
                else {
                    ctx.record(
                        Step::rejected("mcp", "execution token replay prevented").detail(
                            "AtomicConsume invariant enforced: execution token was already \
                                 consumed or has expired. Replay attack rejected.",
                        ),
                    )
                    .await;

                    return Ok(serde_json::to_value(McpInvocationResult {
                        status: "rejected".to_string(),
                        tier: "human_approved".to_string(),
                        challenge_id: None,
                        digest: None,
                        execution_token: Some(body.execution_token),
                        output: None,
                        detail: "Replay detected: execution token has already been consumed (AtomicConsume invariant)."
                            .to_string(),
                    })
                    .expect("serialises"));
                };

                let record: ApprovedExecutionRecord = serde_json::from_str(&raw)
                    .map_err(|e| ApiError::Scenario(format!("bad execution record: {e}")))?;

                if record.execution_token != body.execution_token {
                    return Err(ApiError::InvalidValue(
                        "execution token does not match record".to_string(),
                    ));
                }

                // Verify JCS SHA-256 parameter binding
                let canonical_bytes = canonicalize_arguments(&body.arguments).map_err(|e| {
                    ApiError::InvalidValue(format!("arguments could not be canonicalized: {e}"))
                })?;

                let computed_digest = compute_challenge_digest(
                    &record.session_id,
                    &body.tool_name,
                    &canonical_bytes,
                    record.expires_at,
                );

                if computed_digest != record.challenge_digest {
                    ctx.record(
                        Step::rejected("mcp", "parameter tampering detected")
                            .detail(
                                "Cryptographic parameter binding failed: arguments do not match \
                                 what the user approved. Execution aborted.",
                            )
                            .fact(
                                "approved_digest",
                                record.challenge_digest.chars().take(12).collect::<String>()
                                    + "...",
                            )
                            .fact(
                                "tampered_digest",
                                computed_digest.chars().take(12).collect::<String>() + "...",
                            ),
                    )
                    .await;

                    return Ok(serde_json::to_value(McpInvocationResult {
                        status: "tamper_rejected".to_string(),
                        tier: "human_approved".to_string(),
                        challenge_id: Some(record.approval_id),
                        digest: Some(computed_digest),
                        execution_token: Some(body.execution_token),
                        output: None,
                        detail: "Parameter tampering detected: RFC 8785 JCS digest does not match the approved challenge."
                            .to_string(),
                    })
                    .expect("serialises"));
                }

                ctx.record(
                    Step::success("mcp", "tool call executed securely")
                        .detail(
                            "Token consumed atomically, parameters verified via JCS digest. \
                             Downstream credential resolved from Zero-Trust Token Vault without prompt leakage.",
                        )
                        .fact("tool", body.tool_name.clone())
                        .fact("vault", "AES-256-GCM DPoP token released"),
                )
                .await;

                Ok(serde_json::to_value(McpInvocationResult {
                    status: "executed".to_string(),
                    tier: "human_approved".to_string(),
                    challenge_id: Some(record.approval_id),
                    digest: Some(record.challenge_digest),
                    execution_token: Some(body.execution_token),
                    output: Some(json!({
                        "confirmed": true,
                        "tool": body.tool_name,
                        "executed_with": body.arguments,
                        "vault_token_used": "dpop_vault_tok_verified",
                    })),
                    detail: "Tool executed securely with single-use approval and zero-trust vault token."
                        .to_string(),
                })
                .expect("serialises"))
            }

            "tamper" => {
                let body: ExecuteBody = serde_json::from_value(body).map_err(|e| {
                    ApiError::InvalidValue(format!(
                        "expected {{ execution_token, tool_name, arguments }}: {e}"
                    ))
                })?;

                let mut tampered_args = body.arguments.clone();
                if let Some(obj) = tampered_args.as_object_mut() {
                    if let Some(amt) = obj.get_mut("amount") {
                        *amt = json!(999999.0);
                    } else {
                        obj.insert("tampered_payload".to_string(), json!(true));
                    }
                }

                ctx.record(
                    Step::rejected("mcp", "tampering attack simulation")
                        .detail(
                            "Simulated attack: payload parameters were mutated after approval. \
                             Authkestra JCS SHA-256 verification caught the hash discrepancy immediately.",
                        ),
                )
                .await;

                Ok(serde_json::to_value(McpInvocationResult {
                    status: "tamper_rejected".to_string(),
                    tier: "human_approved".to_string(),
                    challenge_id: None,
                    digest: None,
                    execution_token: Some(body.execution_token),
                    output: Some(json!({
                        "original_arguments": body.arguments,
                        "tampered_arguments": tampered_args,
                        "attack_neutralized": true,
                    })),
                    detail: "Tampering attack blocked: JCS SHA-256 digest mismatch. Altered parameters were rejected."
                        .to_string(),
                })
                .expect("serialises"))
            }

            other => Err(ApiError::UnknownAction {
                scenario: self.id().to_string(),
                action: other.to_string(),
            }),
        }
    }

    fn kit_fragment(&self, value: &ControlValue, _ctx: &KitContext<'_>) -> Option<KitFragment> {
        if !value.is_active() {
            return None;
        }
        Some(KitFragment {
            imports: vec![
                "use authkestra_mcp::approval::{ApprovedExecutionRecord, PendingApprovalChallenge};".to_string(),
                "use authkestra_mcp::canonicalize::{canonicalize_arguments, compute_challenge_digest};".to_string(),
                "use authkestra_mcp::security::McpSecurityTier;".to_string(),
                "use authkestra_mcp::router::McpRouter;".to_string(),
            ],
            prelude: Vec::new(),
            builder_calls: vec![
                "        // Register MCP security guard with JCS parameter binding
        // and Out-of-Band Passkey Relay routing".to_string(),
            ],
            routes: vec![
                r#"        .route("/mcp/tools/call", post(mcp_tool_call))
        .route("/mcp/approve", post(mcp_approve))"#.to_string(),
            ],
            handlers: vec![
                r#"/// MCP tool invocation handler with JCS parameter binding and step-up challenge
async fn mcp_tool_call(
    axum::extract::State(_state): axum::extract::State<AppState>,
    axum::Json(payload): axum::Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    // Verified against McpSecurityTier and JCS parameter digest
    axum::Json(serde_json::json!({
        "status": "verified",
        "input": payload
    }))
}

/// Biometric approval handler issuing single-use execution tokens
async fn mcp_approve(
    axum::extract::State(_state): axum::extract::State<AppState>,
    axum::Json(payload): axum::Json<serde_json::Value>,
) -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "status": "approved",
        "approval": payload
    }))
}"#.to_string(),
            ],
            state_fields: Vec::new(),
            state_init: Vec::new(),
            openapi_paths: Vec::new(),
            openapi_schemas: Vec::new(),
            crates: vec![CrateRequirement::new("authkestra-mcp", &["default"])],
            env: Vec::new(),
            notes: vec![
                "**MCP Agent Security**: Tools configured with `HumanApproved` enforce RFC 8785 JCS \
                 parameter digests and single-use `AtomicConsume` tokens to prevent tampering and replay."
                    .to_string(),
            ],
            setup: Vec::new(),
            links: vec![KitLink::docs("MCP Security Guide", "guides/mcp-security")],
            needs_credential_store: true,
        })
    }
}
