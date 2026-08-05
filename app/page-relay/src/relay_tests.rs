mod tests {
    use super::*;
    use remarque_page_log::{
        ActiveStroke, CommandId, PageCommand, PageDimensions, PageIdentity, PageOperation,
        SharedStroke, StrokeId, SubmittedPageOperation, decode_server_message,
        encode_client_message,
    };
    use remarque_core::stroke::StrokePoint;
    use axum::http::HeaderValue;
    use axum::http::header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    #[test]
    fn secret_hex_round_trips() {
        let secret = [0xa5; 32];
        assert_eq!(decode_hex::<32>(&encode_hex(&secret)), Ok(secret));
    }

    #[test]
    fn guest_palette_never_assigns_black_or_white() {
        for _ in 0..64 {
            let color = choose_guest_color(&[]).unwrap();
            assert!(!matches!(color, Color::Black | Color::White));
        }
    }

    #[test]
    fn startup_commits_nonempty_strokes_left_active_without_a_connection() {
        let owner = Participant {
            id: ParticipantId::from_bytes([1; 16]),
            role: ParticipantRole::Owner,
            color: Color::Black,
        };
        let stroke_id = StrokeId::from_bytes([2; 16]);
        let point = StrokePoint {
            x: 12.0,
            y: 34.0,
            two_segment_distance_quarters: 0,
            width_quarter_pixels: 8,
            direction: 0,
            pressure: 128,
        };
        let journal = PageJournal::from_snapshot(PageSnapshot {
            identity: PageIdentity {
                document_id: "notebook-1".to_owned(),
                page_index: 0,
            },
            dimensions: PageDimensions {
                width: 100,
                height: 200,
            },
            background: None,
            strokes: Vec::new(),
            active_strokes: vec![ActiveStroke {
                stroke: SharedStroke {
                    id: stroke_id,
                    author: owner.id,
                    color: owner.color,
                    points: vec![point],
                },
            }],
            revision: 0,
        })
        .unwrap();
        let mut stored = StoredShare {
            id: ShareId::from_bytes([3; 16]),
            expires_at_unix_seconds: u64::MAX,
            guest_secret_digest: [4; 32],
            owner,
            participants: vec![owner],
            sessions: Vec::new(),
            journal,
            revoked: false,
        };

        assert!(finalize_orphaned_strokes(&mut stored).unwrap());
        assert!(stored.journal.snapshot().active_strokes.is_empty());
        assert_eq!(stored.journal.snapshot().strokes.len(), 1);
        assert_eq!(stored.journal.snapshot().strokes[0].points, vec![point]);
        assert_eq!(stored.journal.snapshot().revision, 1);
    }

    #[tokio::test]
    async fn guest_stroke_is_relayed_to_owner_with_assigned_identity() {
        let root =
            std::env::temp_dir().join(format!("remarque-page-relay-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let settings = RelaySettings {
            listen_address: "127.0.0.1:0".parse().unwrap(),
            public_origin: "https://remarque.geiger.ink".to_owned(),
            data_directory: root.clone(),
            viewer_directory: root.join("wasm"),
            owner_token: "relay-owner-token-with-at-least-32-bytes".to_owned(),
        };
        let state = RelayState::load(settings.clone()).unwrap();
        let mut owner_headers = HeaderMap::new();
        owner_headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer relay-owner-token-with-at-least-32-bytes"),
        );
        let created = create_share(
            State(state.clone()),
            owner_headers,
            Json(CreateShareRequest {
                snapshot: PageSnapshot {
                    identity: PageIdentity {
                        document_id: "notebook-1".to_owned(),
                        page_index: 0,
                    },
                    dimensions: PageDimensions {
                        width: 100,
                        height: 200,
                    },
                    background: None,
                    strokes: Vec::new(),
                    active_strokes: Vec::new(),
                    revision: 0,
                },
            }),
        )
        .await
        .unwrap()
        .0;
        let secret = created.guest_url.rsplit('.').next().unwrap().to_owned();
        let response = redeem_share(
            Path(created.share_id.clone()),
            State(state.clone()),
            Json(RedeemShareRequest {
                secret: secret.clone(),
                session_token: None,
            }),
        )
        .await
        .unwrap()
        .0;
        let resumed = redeem_share(
            Path(created.share_id.clone()),
            State(state.clone()),
            Json(RedeemShareRequest {
                secret: secret.clone(),
                session_token: Some(response.session_token.clone()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(resumed.participant, response.participant);
        assert_eq!(resumed.session_token, response.session_token);
        let second_tab = redeem_share(
            Path(created.share_id.clone()),
            State(state.clone()),
            Json(RedeemShareRequest {
                secret,
                session_token: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_ne!(response.participant.id, second_tab.participant.id);
        assert_ne!(response.participant.color, second_tab.participant.color);
        assert_ne!(response.session_token, second_tab.session_token);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, page_relay_router(state)).await
        });
        let websocket_url = format!("ws://{address}/api/shares/{}/ws", created.share_id);
        let mut owner_request = websocket_url.clone().into_client_request().unwrap();
        owner_request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", created.owner_token)).unwrap(),
        );
        let (mut owner_socket, _) = connect_async(owner_request).await.unwrap();
        let mut guest_request = websocket_url.into_client_request().unwrap();
        let guest_protocol = format!(
            "{GUEST_SESSION_PROTOCOL_PREFIX}{}",
            response.session_token
        );
        guest_request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_str(&guest_protocol).unwrap(),
        );
        let (mut guest_socket, guest_response) = connect_async(guest_request).await.unwrap();
        assert_eq!(
            guest_response.headers()[SEC_WEBSOCKET_PROTOCOL],
            guest_protocol
        );
        let owner_welcome =
            decode_server_message(&owner_socket.next().await.unwrap().unwrap().into_data())
                .unwrap();
        assert!(matches!(owner_welcome, ServerMessage::Welcome { .. }));
        let guest =
            match decode_server_message(&guest_socket.next().await.unwrap().unwrap().into_data())
                .unwrap()
            {
                ServerMessage::Welcome { participant, .. } => participant,
                message => panic!("unexpected guest message: {message:?}"),
            };
        let stroke_id = StrokeId::from_bytes([2; 16]);
        let command = PageCommand {
            id: CommandId::from_bytes([3; 16]),
            operation: SubmittedPageOperation::BeginStroke { stroke_id },
        };
        guest_socket
            .send(tokio_tungstenite::tungstenite::Message::Binary(
                encode_client_message(&ClientMessage::Submit { command })
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        for socket in [&mut owner_socket, &mut guest_socket] {
            let message =
                decode_server_message(&socket.next().await.unwrap().unwrap().into_data()).unwrap();
            match message {
                ServerMessage::Applied { operation } => match operation.operation {
                    PageOperation::BeginStroke { stroke } => {
                        assert_eq!(stroke.author, guest.id);
                        assert_eq!(stroke.color, guest.color);
                        assert!(!matches!(stroke.color, Color::Black | Color::White));
                    }
                    operation => panic!("unexpected operation: {operation:?}"),
                },
                message => panic!("unexpected relay message: {message:?}"),
            }
        }
        let point = StrokePoint {
            x: 12.0,
            y: 34.0,
            two_segment_distance_quarters: 0,
            width_quarter_pixels: 8,
            direction: 0,
            pressure: 128,
        };
        for (command_id, operation) in [
            (
                4,
                SubmittedPageOperation::AppendStrokePoints {
                    stroke_id,
                    first_point: 0,
                    points: vec![point],
                },
            ),
            (5, SubmittedPageOperation::CommitStroke { stroke_id }),
        ] {
            guest_socket
                .send(tokio_tungstenite::tungstenite::Message::Binary(
                    encode_client_message(&ClientMessage::Submit {
                        command: PageCommand {
                            id: CommandId::from_bytes([command_id; 16]),
                            operation,
                        },
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            for socket in [&mut owner_socket, &mut guest_socket] {
                assert!(matches!(
                    decode_server_message(&socket.next().await.unwrap().unwrap().into_data())
                        .unwrap(),
                    ServerMessage::Applied { .. }
                ));
                if command_id == 5 {
                    assert!(matches!(
                        decode_server_message(&socket.next().await.unwrap().unwrap().into_data())
                            .unwrap(),
                        ServerMessage::Digest { .. }
                    ));
                }
            }
        }
        let abandoned_stroke_id = StrokeId::from_bytes([6; 16]);
        for (command_id, operation) in [
            (
                7,
                SubmittedPageOperation::BeginStroke {
                    stroke_id: abandoned_stroke_id,
                },
            ),
            (
                8,
                SubmittedPageOperation::AppendStrokePoints {
                    stroke_id: abandoned_stroke_id,
                    first_point: 0,
                    points: vec![point],
                },
            ),
        ] {
            guest_socket
                .send(tokio_tungstenite::tungstenite::Message::Binary(
                    encode_client_message(&ClientMessage::Submit {
                        command: PageCommand {
                            id: CommandId::from_bytes([command_id; 16]),
                            operation,
                        },
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            for socket in [&mut owner_socket, &mut guest_socket] {
                assert!(matches!(
                    decode_server_message(&socket.next().await.unwrap().unwrap().into_data())
                        .unwrap(),
                    ServerMessage::Applied { .. }
                ));
            }
        }
        guest_socket.close(None).await.unwrap();
        let cleanup = tokio::time::timeout(Duration::from_secs(2), owner_socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            decode_server_message(&cleanup.into_data()).unwrap(),
            ServerMessage::Applied {
                operation: remarque_page_log::AppliedPageOperation {
                    operation: PageOperation::CommitStroke { stroke_id },
                    ..
                }
            } if stroke_id == abandoned_stroke_id
        ));
        assert!(matches!(
            decode_server_message(
                &tokio::time::timeout(Duration::from_secs(2), owner_socket.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .into_data()
            )
            .unwrap(),
            ServerMessage::Digest { .. }
        ));
        drop(owner_socket);
        drop(guest_socket);
        server.abort();
        let reloaded = RelayState::load(settings).unwrap();
        let share = reloaded
            .share(created.share_id.parse().unwrap())
            .unwrap();
        let stored = share
            .stored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(stored.journal.snapshot().strokes.len(), 2);
        assert!(stored
            .journal
            .snapshot()
            .strokes
            .iter()
            .all(|stroke| stroke.points == vec![point]));
        assert!(stored.journal.snapshot().active_strokes.is_empty());
        drop(stored);
        let _ = std::fs::remove_dir_all(root);
    }
}
