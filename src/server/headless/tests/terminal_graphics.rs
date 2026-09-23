use super::*;

const IMAGE: &[u8] = b"\x1b_Ga=T,f=32,t=d,i=7,p=3,s=1,v=1,c=1,r=1,q=2;/wAA/w==\x1b\\";

fn frame_bytes(receiver: &std::sync::mpsc::Receiver<Vec<u8>>) -> String {
    let ServerMessage::Terminal(frame) = read_server_message(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("terminal frame"),
    ) else {
        panic!("expected ANSI terminal frame");
    };
    String::from_utf8(frame.bytes).expect("ASCII frame")
}

fn connect(
    server: &mut HeadlessServer,
    target: String,
    observe: bool,
    pixels: u32,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (writer, _control, render) = test_client_writer();
    server.handle_server_event(ServerEvent::ClientConnected {
        client_id: 7,
        cols: 80,
        rows: 24,
        cell_width_px: pixels,
        cell_height_px: pixels,
        pixel_mouse: false,
        writer,
    });
    if observe {
        server.handle_server_event(ServerEvent::ClientObserveTerminal {
            client_id: 7,
            target,
        });
    } else {
        server.handle_server_event(ServerEvent::ClientAttachTerminal {
            client_id: 7,
            terminal_id: target,
            takeover: false,
        });
    }
    render
}

#[test]
fn terminal_graphics_attach_and_observe_upload_repaint_and_remove_images() {
    for observe in [false, true] {
        with_terminal_session_test_server(|server, terminal_id, target, _| {
            let frames = connect(server, target, observe, 16);
            server
                .app
                .terminal_runtimes
                .get(&terminal_id)
                .unwrap()
                .test_process_pty_bytes(IMAGE);
            server.render_and_stream();
            let first = frame_bytes(&frames);
            let mut host = crate::ghostty::Terminal::new(80, 24, 0).unwrap();
            host.enable_kitty_graphics().unwrap();
            host.resize(80, 24, 16, 16).unwrap();
            host.write(first.as_bytes());
            let images = host.kitty_image_placements().unwrap();
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].data, [255, 0, 0, 255]);
            assert!(first.contains("a=t"));
            assert!(first.contains("a=p"));
            assert!(first.contains("/wAA/w=="));
            assert!(first.find("a=p").unwrap() < first.rfind("\x1b[?2026l").unwrap());

            server.clients.get_mut(&7).unwrap().request_repaint();
            server.render_and_stream();
            let repaint = frame_bytes(&frames);
            assert!(!repaint.contains("a=t"));
            assert!(repaint.contains("a=p"));
            host.write(repaint.as_bytes());
            assert_eq!(host.kitty_image_placements().unwrap().len(), 1);

            server
                .app
                .terminal_runtimes
                .get(&terminal_id)
                .unwrap()
                .test_process_pty_bytes(b"\x1b_Ga=d,d=A\x1b\\");
            server.clients.get_mut(&7).unwrap().request_repaint();
            server.render_and_stream();
            let deleted = frame_bytes(&frames);
            assert!(deleted.contains("a=d"));
            assert!(!deleted.contains("a=p"));
            host.write(deleted.as_bytes());
            assert!(host.kitty_image_placements().unwrap().is_empty());
        });
    }
}

#[test]
fn terminal_graphics_retry_preserves_unsent_uploads() {
    with_terminal_session_test_server(|server, terminal_id, target, _| {
        let frames = connect(server, target, false, 16);
        server.render_and_stream();
        server
            .app
            .terminal_runtimes
            .get(&terminal_id)
            .unwrap()
            .test_process_pty_bytes(IMAGE);
        server.clients.get_mut(&7).unwrap().request_repaint();
        server.render_and_stream();
        assert!(!frame_bytes(&frames).contains("a=t"));
        server.render_and_stream();
        let retry = frame_bytes(&frames);
        assert!(retry.contains("a=t"));
        assert!(retry.contains("a=p"));
    });
}

#[test]
fn terminal_graphics_wait_for_cell_dimensions() {
    with_terminal_session_test_server(|server, terminal_id, target, _| {
        let frames = connect(server, target, false, 0);
        server
            .app
            .terminal_runtimes
            .get(&terminal_id)
            .unwrap()
            .test_process_pty_bytes(IMAGE);
        server.render_and_stream();
        assert!(!frame_bytes(&frames).contains("a=t"));
        let client = server.clients.get_mut(&7).unwrap();
        client.cell_size = crate::kitty_graphics::HostCellSize {
            width_px: 8,
            height_px: 16,
        };
        client.request_repaint();
        server.render_and_stream();
        assert!(frame_bytes(&frames).contains("a=t"));
    });
}
