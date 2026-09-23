use super::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct TerminalGraphics {
    terminal_id: String,
    cache: HostGraphicsCache,
}

impl TerminalGraphics {
    pub(crate) fn prepare(
        &self,
        terminal_id: &str,
        runtime: &crate::terminal::TerminalRuntime,
        area: Rect,
        cell_size: HostCellSize,
    ) -> (Self, EncodedGraphics) {
        let mut next = self.clone();
        let mut bytes = Vec::new();
        if next.terminal_id != terminal_id {
            bytes.extend(next.cache.clear_bytes());
            next.terminal_id = terminal_id.to_owned();
        }
        if !is_enabled() || !cell_size.is_known() {
            bytes.extend(next.cache.clear_bytes());
            return (
                next,
                EncodedGraphics {
                    bytes,
                    incomplete: false,
                },
            );
        }
        let pane_id = PaneId::from_raw(0);
        let mut requested = HashSet::new();
        let scrollback_offset = runtime
            .scroll_metrics()
            .map(|metrics| metrics.offset_from_bottom as u32)
            .unwrap_or(0);
        let placements = runtime
            .kitty_image_placements_with_data_filter(|descriptor| {
                terminal_image_needs_data(
                    pane_id,
                    descriptor,
                    &next.cache.images,
                    &next.cache.oversized,
                    &mut requested,
                )
            })
            .into_iter()
            .map(|placement| HostPlacement {
                pane_id,
                host_image_id: None,
                area,
                cell_size,
                source_key: HostSourceKey::Terminal {
                    pane_id,
                    image_id: placement.image_id,
                },
                placement,
                scrollback_offset,
            })
            .collect::<Vec<_>>();
        next.cache.request_placement_replay();
        let encoded = encode_graphics_update_incremental(
            &mut next.cache,
            &placements,
            &HashSet::new(),
            Some(HEADLESS_GRAPHICS_TRANSACTION_BUDGET),
            true,
        );
        bytes.extend(encoded.bytes);
        (
            next,
            EncodedGraphics {
                bytes,
                incomplete: encoded.incomplete,
            },
        )
    }
}
