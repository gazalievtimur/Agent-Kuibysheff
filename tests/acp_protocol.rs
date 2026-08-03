//! ACP protocol smoke test over an in-process duplex channel.

use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, SessionId, SessionNotification,
    SessionUpdate, StopReason, TextContent,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{Agent, Channel, Client, ConnectionTo, Error as AcpError};
use tokio::sync::Mutex;

#[tokio::test]
async fn acp_initialize_session_prompt_roundtrip() {
    let (agent_side, client_side) = Channel::duplex();
    let updates: Arc<Mutex<Vec<SessionUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let updates_client = Arc::clone(&updates);

    let agent_task = tokio::spawn(async move {
        Agent
            .builder()
            .name("test-agent")
            .on_receive_request(
                async move |req: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(req.protocol_version)
                            .agent_capabilities(AgentCapabilities::new()),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_req: NewSessionRequest, responder, _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new("sess-1")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |req: PromptRequest, responder, cx| {
                    cx.send_notification(SessionNotification::new(
                        req.session_id.clone(),
                        SessionUpdate::AgentMessageChunk(
                            agent_client_protocol::schema::v1::ContentChunk::new(
                                ContentBlock::Text(TextContent::new("hello from agent")),
                            ),
                        ),
                    ))?;
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(agent_side)
            .await
    });

    let client_result = Client
        .builder()
        .name("test-client")
        .on_receive_notification(
            async move |notif: SessionNotification, _cx| {
                updates_client.lock().await.push(notif.update);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(client_side, |cx: ConnectionTo<Agent>| async move {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;

            let session = cx
                .send_request(NewSessionRequest::new(std::env::temp_dir()))
                .block_task()
                .await?;

            let prompt_response = cx
                .send_request(PromptRequest::new(
                    session.session_id,
                    vec![ContentBlock::Text(TextContent::new("ping"))],
                ))
                .block_task()
                .await?;

            assert_eq!(prompt_response.stop_reason, StopReason::EndTurn);
            Ok::<(), AcpError>(())
        })
        .await;

    client_result.expect("client flow");
    agent_task.await.expect("join").expect("agent flow");

    let captured = updates.lock().await;
    assert!(
        captured
            .iter()
            .any(|u| matches!(u, SessionUpdate::AgentMessageChunk(_))),
        "expected agent message chunk update, got {captured:?}"
    );
}
