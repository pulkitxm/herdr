use super::*;
use std::sync::mpsc::Receiver;

fn connect_graphics_terminal(
    server: &mut HeadlessServer,
    client_id: u64,
    target: &str,
    observe: bool,
) -> (Receiver<Vec<u8>>, Receiver<Vec<u8>>) {
    let (writer, control, render) = test_client_writer();
    server.handle_server_event(ServerEvent::ClientConnected {
        client_id,
        cols: 80,
        rows: 24,
        cell_width_px: 10,
        cell_height_px: 20,
        pixel_mouse: false,
        writer,
    });
    if observe {
        assert!(server.observe_terminal_client(client_id, target.to_owned()));
    } else {
        assert!(server.control_terminal_client(client_id, target.to_owned(), true));
    }
    (control, render)
}

fn terminal_bytes(render: &Receiver<Vec<u8>>) -> Vec<u8> {
    let ServerMessage::Terminal(frame) =
        read_server_message(render.recv_timeout(Duration::from_secs(1)).unwrap())
    else {
        panic!("expected terminal frame");
    };
    frame.bytes
}

fn has_kitty(bytes: &[u8]) -> bool {
    bytes.windows(3).any(|part| part == b"\x1b_G")
}

const RED_IMAGE: &[u8] = b"\x1b[3;5H\x1b_Ga=T,f=32,s=1,v=1,i=7,p=3,c=2,r=2,q=2;/wAA/w==\x1b\\";

#[test]
fn terminal_graphics_replays_real_image_to_controller_and_observer() {
    with_terminal_session_test_server(|server, terminal, target, _| {
        server.app.state.kitty_graphics_enabled = true;
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(RED_IMAGE);
        let (_controller_control, controller) =
            connect_graphics_terminal(server, 7, &target, false);
        let (_observer_control, observer) = connect_graphics_terminal(server, 8, &target, true);
        server.render_and_stream();
        for render in [&controller, &observer] {
            let bytes = terminal_bytes(render);
            assert!(has_kitty(&bytes));
            let text = String::from_utf8_lossy(&bytes);
            assert!(text.contains("a=t"), "image must be uploaded: {text}");
            assert!(text.contains("/wAA/w=="), "exact synthetic pixels");
            assert!(text.contains("a=p"), "image must be placed");
            assert!(text.contains("c=2,r=2"), "placement geometry");
        }
        server.render_and_stream();
        for render in [&controller, &observer] {
            let bytes = terminal_bytes(render);
            let text = String::from_utf8_lossy(&bytes);
            assert!(text.contains("a=p"));
            assert!(
                !text.contains("a=t"),
                "cached pixels must not be uploaded again"
            );
        }
    });
}

#[test]
fn terminal_graphics_updates_without_text_and_cleans_deleted_images() {
    with_terminal_session_test_server(|server, terminal, target, _| {
        server.app.state.kitty_graphics_enabled = true;
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(RED_IMAGE);
        let (_control, render) = connect_graphics_terminal(server, 7, &target, false);
        server.render_and_stream();
        terminal_bytes(&render);
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(
                b"\x1b[3;5H\x1b_Ga=T,f=32,s=1,v=1,i=7,p=3,c=2,r=2,q=2;AP8A/w==\x1b\\",
            );
        server.render_and_stream();
        let bytes = terminal_bytes(&render);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("a=t"));
        assert!(text.contains("AP8A/w=="));
        assert!(text.contains("a=d,d=I"));
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(b"\x1b_Ga=d,d=I,i=7,q=2;\x1b\\");
        server.render_and_stream();
        let bytes = terminal_bytes(&render);
        assert!(String::from_utf8_lossy(&bytes).contains("a=d,d=I"));
        assert!(!String::from_utf8_lossy(&bytes).contains("a=p"));
        server.render_and_stream();
        assert!(render.try_recv().is_err());
    });
}

#[test]
fn terminal_graphics_reattach_and_resize_use_connection_local_geometry() {
    with_terminal_session_test_server(|server, terminal, target, _| {
        server.app.state.kitty_graphics_enabled = true;
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(RED_IMAGE);
        let (_control, render) = connect_graphics_terminal(server, 7, &target, false);
        server.render_and_stream();
        let first = terminal_bytes(&render);
        assert!(server.handle_server_event(ServerEvent::ClientResize {
            client_id: 7,
            cols: 5,
            rows: 4,
            cell_width_px: 8,
            cell_height_px: 16,
            pixel_mouse: false,
        }));
        server.render_and_stream();
        let resized = terminal_bytes(&render);
        let text = String::from_utf8_lossy(&resized);
        assert!(text.contains("a=p"));
        assert!(text.contains("c=2,r=2"), "resized placement: {text}");
        assert!(
            text.contains("\x1b[2;4H"),
            "placement must follow terminal reflow"
        );
        assert!(!text.contains("a=t"));
        assert_eq!(
            server
                .app
                .terminal_runtimes
                .get(&terminal)
                .unwrap()
                .pixel_size(),
            Some((40, 64))
        );
        server.remove_client_and_resize_if_needed(7);
        assert!(!server.terminal_attach_owners.contains_key(&target));
        let (_control, reattached) = connect_graphics_terminal(server, 8, &target, false);
        server.render_and_stream();
        let bytes = terminal_bytes(&reattached);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("a=t"));
        assert!(text.contains("/wAA/w=="));
        assert!(text.contains("c=2,r=2"));
        assert_ne!(
            first, bytes,
            "separate connections must have distinct image namespaces"
        );
    });
}

#[test]
fn terminal_graphics_failed_send_does_not_commit_image_delivery() {
    with_terminal_session_test_server(|server, terminal, target, _| {
        server.app.state.kitty_graphics_enabled = true;
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(RED_IMAGE);
        let (_control, render) = connect_graphics_terminal(server, 7, &target, false);
        server.clients[&7]
            .writer
            .as_ref()
            .unwrap()
            .render
            .try_send(vec![42])
            .unwrap();
        server.render_and_stream();
        assert_eq!(server.clients[&7].deferred_render(), DeferredRender::Full);
        assert_eq!(render.recv().unwrap(), [42]);
        server.render_and_stream();
        let bytes = terminal_bytes(&render);
        assert!(String::from_utf8_lossy(&bytes).contains("a=t"));
        assert!(String::from_utf8_lossy(&bytes).contains("/wAA/w=="));
        assert_eq!(server.clients[&7].deferred_render(), DeferredRender::None);
    });
}

#[tokio::test]
async fn terminal_graphics_two_panes_do_not_change_shared_focus_or_zoom() {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("graphics");
    let first = workspace.focused_pane_id().unwrap();
    let second = workspace.test_split(ratatui::layout::Direction::Horizontal);
    for (pane, bytes) in [
        (first, RED_IMAGE),
        (
            second,
            b"\x1b[3;5H\x1b_Ga=T,f=32,s=1,v=1,i=7,p=3,c=2,r=2,q=2;AP8A/w==\x1b\\".as_slice(),
        ),
    ] {
        server.app.terminal_runtimes.insert(
            workspace.terminal_id(pane).unwrap().clone(),
            crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 24, bytes),
        );
    }
    server.app.state.workspaces = vec![workspace];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.kitty_graphics_enabled = true;
    let focused = server.app.state.workspaces[0].tabs[0].layout.focused();
    let first_target = server.app.public_pane_id(0, first).unwrap();
    let second_target = server.app.public_pane_id(0, second).unwrap();
    let (_first_control, first_render) =
        connect_graphics_terminal(&mut server, 7, &first_target, false);
    let (_second_control, second_render) =
        connect_graphics_terminal(&mut server, 8, &second_target, false);
    server.render_and_stream();
    let first_bytes = terminal_bytes(&first_render);
    let second_bytes = terminal_bytes(&second_render);
    assert!(String::from_utf8_lossy(&first_bytes).contains("/wAA/w=="));
    assert!(!String::from_utf8_lossy(&first_bytes).contains("AP8A/w=="));
    assert!(String::from_utf8_lossy(&second_bytes).contains("AP8A/w=="));
    assert!(!String::from_utf8_lossy(&second_bytes).contains("/wAA/w=="));
    assert_eq!(
        server.app.state.workspaces[0].tabs[0].layout.focused(),
        focused
    );
    assert!(!server.app.state.workspaces[0].tabs[0].zoomed);
    assert_eq!(server.app.state.active, Some(0));
    assert_eq!(
        server.app.state.workspaces[0].tabs[0]
            .layout
            .pane_ids()
            .len(),
        2
    );
    shutdown_test_runtimes(&mut server);
}

#[test]
fn terminal_graphics_unknown_pixels_hide_and_restore_cached_image() {
    with_terminal_session_test_server(|server, terminal, target, _| {
        server.app.state.kitty_graphics_enabled = true;
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(RED_IMAGE);
        let (_control, render) = connect_graphics_terminal(server, 7, &target, false);
        server.render_and_stream();
        terminal_bytes(&render);
        server.clients.get_mut(&7).unwrap().cell_size = Default::default();
        server.render_and_stream();
        let bytes = terminal_bytes(&render);
        assert!(String::from_utf8_lossy(&bytes).contains("a=d,d=I"));
        server.clients.get_mut(&7).unwrap().cell_size = crate::kitty_graphics::HostCellSize {
            width_px: 10,
            height_px: 20,
        };
        server.render_and_stream();
        assert!(String::from_utf8_lossy(&terminal_bytes(&render)).contains("a=t"));
        server.app.state.kitty_graphics_enabled = false;
        server.render_and_stream();
        assert!(String::from_utf8_lossy(&terminal_bytes(&render)).contains("a=d,d=I"));
    });
}

#[cfg(unix)]
#[test]
fn terminal_graphics_file_backed_pixels_use_inline_transport() {
    use base64::Engine as _;
    with_terminal_session_test_server(|server, terminal, target, _| {
        server.app.state.kitty_graphics_enabled = true;
        let store = crate::pane_graphics_files::FileStore::native_sources();
        let pixels = [17, 34, 51, 255];
        let source = store.export(&pixels).unwrap();
        let path = base64::engine::general_purpose::STANDARD
            .encode(source.path().as_os_str().as_encoded_bytes());
        let runtime = server.app.terminal_runtimes.get(&terminal).unwrap();
        runtime.test_enable_kitty_source_forwarding();
        runtime.test_process_pty_bytes(
            format!("\x1b_Ga=T,f=32,t=f,s=1,v=1,i=7,p=3,c=2,r=2,q=2;{path}\x1b\\").as_bytes(),
        );
        let (_control, render) = connect_graphics_terminal(server, 7, &target, false);
        server.render_and_stream();
        let bytes = terminal_bytes(&render);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains(&base64::engine::general_purpose::STANDARD.encode(pixels)));
        assert!(
            !text.contains("t=f"),
            "SSH clients cannot use server file paths"
        );
        server.render_and_stream();
        assert!(!String::from_utf8_lossy(&terminal_bytes(&render)).contains("a=t"));
    });
}

#[tokio::test]
#[ignore = "manual fixed-geometry direct attachment profile with 1/15 populated panes"]
async fn terminal_attachment_render_scale_profile() {
    for count in [1, 15] {
        for graphics in [false, true] {
            let mut server = test_headless_server();
            let mut workspace = crate::workspace::Workspace::test_new("profile");
            let root = workspace.focused_pane_id().unwrap();
            server.app.terminal_runtimes.insert(
                workspace.terminal_id(root).unwrap().clone(),
                crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 24, RED_IMAGE),
            );
            for _ in 1..count {
                let pane = workspace.test_split(ratatui::layout::Direction::Horizontal);
                server.app.terminal_runtimes.insert(
                    workspace.terminal_id(pane).unwrap().clone(),
                    crate::terminal::TerminalRuntime::test_with_screen_bytes(
                        80,
                        24,
                        b"synthetic populated terminal\r\nsecond row\r\nthird row",
                    ),
                );
            }
            server.app.state.workspaces = vec![workspace];
            server.app.state.ensure_test_terminals();
            server.app.state.active = Some(0);
            server.app.state.kitty_graphics_enabled = graphics;
            let target = server.app.public_pane_id(0, root).unwrap();
            let (_control, render) = connect_graphics_terminal(&mut server, 7, &target, false);
            let mut samples = Vec::new();
            for sample in 0..55 {
                server.clients.get_mut(&7).unwrap().request_repaint();
                let started = Instant::now();
                server.render_and_stream();
                let elapsed = started.elapsed().as_nanos();
                let bytes = terminal_bytes(&render);
                assert_eq!(has_kitty(&bytes), graphics);
                if sample >= 5 {
                    samples.push(elapsed);
                }
            }
            samples.sort_unstable();
            eprintln!(
                "direct attachment panes={count} graphics={graphics} median_ns={} p95_ns={}",
                samples[25], samples[47]
            );
            shutdown_test_runtimes(&mut server);
        }
    }
}

#[test]
fn terminal_graphics_large_upload_uses_graphics_frame_limit_without_corruption() {
    use base64::Engine as _;
    with_terminal_session_test_server(|server, terminal, target, _| {
        server.app.state.kitty_graphics_enabled = true;
        let pixels = [23, 42, 81, 255].repeat(750 * 600);
        let mut input = Vec::new();
        crate::kitty_graphics::write_kitty_data(
            &mut input,
            "a=T,f=32,s=750,v=600,i=7,p=3,c=2,r=2,q=2",
            &pixels,
        )
        .unwrap();
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(&input);
        let (_control, render) = connect_graphics_terminal(server, 7, &target, false);
        server.render_and_stream();
        let framed = render.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(framed.len() > MAX_FRAME_SIZE);
        let ServerMessage::Terminal(frame) =
            protocol::read_message(&mut std::io::Cursor::new(framed), MAX_GRAPHICS_FRAME_SIZE)
                .unwrap()
        else {
            panic!("expected graphics-sized terminal frame");
        };
        let command = regex::bytes::Regex::new(r"\x1b_G[^;]*;([^\x1b]*)\x1b\\").unwrap();
        let payload = command
            .captures_iter(&frame.bytes)
            .flat_map(|capture| capture[1].to_vec())
            .collect::<Vec<_>>();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(payload)
                .unwrap(),
            pixels
        );
    });
}
