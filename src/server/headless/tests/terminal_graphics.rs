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

#[test]
fn terminal_graphics_replays_real_image_to_controller_and_observer() {
    with_terminal_session_test_server(|server, terminal, target, _| {
        server.app.state.kitty_graphics_enabled = true;
        server
            .app
            .terminal_runtimes
            .get(&terminal)
            .unwrap()
            .test_process_pty_bytes(
                b"\x1b[3;5H\x1b_Ga=T,f=32,s=1,v=1,i=7,p=3,c=2,r=2,q=2;/wAA/w==\x1b\\",
            );
        let (_control, controller) = connect_graphics_terminal(server, 7, &target, false);
        let (_control, observer) = connect_graphics_terminal(server, 8, &target, true);
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
