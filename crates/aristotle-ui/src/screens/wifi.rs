// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wi-Fi: the switch with the connection state, the network policy the
//! device agent applies, and a paged list of the networks nearby with
//! their signal, a lock for secured ones and "Forget" for saved ones.
//!
//! The screen shows plain data ([`WifiView`]); the app talks to the stock
//! `wifid` (`aristotle_hal::wifi`) on a worker thread and rebuilds the
//! screen with `Refresh::Auto` when something changes, so each change is
//! one partial update of the pixels that differ.

use super::library::{page_of, status_line, StatusLine};
use super::{icon_button, text_button};
use crate::geom::{Insets, Rect, Size, MIN_TARGET};
use crate::icons::Icon;
use crate::text::{Align, Paragraph, TextEngine};
use crate::theme::{margin, IconSize, Space, Styles, INK, PAPER, RULE};
use crate::tree::{
    pages, ActionId, Constraints, Cross, Feedback, Kind, Node, NodeId, PaintCx, Tree, Widget,
};
use crate::widgets::Toggle;
use aristotle_i18n::{tr, trn, Msg, Plural};

pub mod action {
    use crate::tree::ActionId;
    pub const BACK: ActionId = 1;
    /// The Wi-Fi switch.
    pub const TOGGLE: ActionId = 2;
    pub const SCAN: ActionId = 3;
    pub const PREV_PAGE: ActionId = 5;
    pub const NEXT_PAGE: ActionId = 6;
    /// `NETWORK + i` taps `networks[i]`.
    pub const NETWORK: ActionId = 100;
    /// `FORGET + i` forgets `networks[i]`.
    pub const FORGET: ActionId = 400;
}

/// The network policy, as `aristotle_hal::wifi::Policy` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// No rules of Aristotle's: the stock firewall.
    Stock,
    /// The agent's allowlist. `no_probe`: `WIFI_NO_NET_PROBE` is present.
    Scoped { hosts: usize, no_probe: bool },
}

/// One network in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkRow<'a> {
    pub name: &'a str,
    /// 0 to 5.
    pub signal: u8,
    pub secured: bool,
    /// `wifid` has a profile for it.
    pub saved: bool,
    pub connected: bool,
    /// Enterprise or anything else this screen cannot set up.
    pub supported: bool,
    /// "Forget" was tapped once: it now asks "Forget?".
    pub forget_armed: bool,
}

/// What the list area shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListState {
    /// Wi-Fi is off: a hint to turn it on.
    Off,
    /// A scan runs and nothing was found before it.
    Scanning,
    Networks,
}

/// Everything the screen shows.
pub struct WifiView<'a> {
    pub status: StatusLine<'a>,
    /// The switch's position.
    pub on: bool,
    /// The line under "Wi-Fi": the state, or what went wrong.
    pub state: &'a str,
    /// Something slow is under way: the busy mark is shown.
    pub busy: bool,
    pub list: ListState,
    pub networks: &'a [NetworkRow<'a>],
    pub policy: Policy,
}

/// Handles to the nodes the app reads after layout.
pub struct WifiScreen {
    pub list: NodeId,
    pub pager_label: NodeId,
    pub prev: NodeId,
    pub next: NodeId,
    pub busy: NodeId,
    /// The switch row.
    pub toggle: NodeId,
}

/// Signal strength as five bars of rising height, filled up to the level.
/// Pure black and white.
pub struct SignalBars {
    pub level: u8,
}

impl Widget for SignalBars {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(Size::new(48, 40))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let n = 5;
        let gap = 3;
        let w = (r.w - gap * (n - 1)) / n;
        for i in 0..n {
            let h = r.h * (i + 1) / n;
            let bar = Rect::new(r.x + i * (w + gap), r.bottom() - h, w, h);
            if (i as u8) < self.level {
                cx.painter.fill_rect(bar, INK);
            } else {
                cx.painter.fill_rect(bar, INK);
                cx.painter.fill_rect(bar.outset(-2), PAPER);
            }
        }
    }
}

fn heading(t: &mut Tree, s: &Styles, text: &str) -> NodeId {
    let h = t.label(text, s.caps());
    t.pad(Insets::new(Space::L, 0, Space::XS, 0), h)
}

fn network_row(t: &mut Tree, s: &Styles, i: usize, n: &NetworkRow) -> NodeId {
    let bars = t.custom(SignalBars { level: n.signal });
    let mut name = Paragraph::plain(n.name, s.item(), Align::Left);
    name.style.max_lines = Some(1);
    let name = t.text(name);
    let detail = if n.connected {
        tr(Msg::WifiInUse)
    } else if !n.supported {
        tr(Msg::WifiUnsupported)
    } else if n.saved {
        tr(Msg::WifiSaved)
    } else if !n.secured {
        tr(Msg::WifiOpen)
    } else {
        ""
    };
    let mut words = vec![name];
    if !detail.is_empty() {
        words.push(t.label(detail, s.small()));
    }
    let words = t.add(Node::column(2).flex(1), words);
    let mut children = vec![bars, words];
    if n.secured {
        children.push(t.icon(Icon::Lock, IconSize::SMALL, INK));
    }
    if n.saved {
        let word = if n.forget_armed {
            tr(Msg::WifiForgetConfirm)
        } else {
            tr(Msg::WifiForget)
        };
        let style = s.ui_bold();
        let l = t.label(word, style);
        let p = t.pad(Insets::vh(0, 20), l);
        let b = t.add(
            Node::new(Kind::MinSize(Size::new(MIN_TARGET, MIN_TARGET)))
                .border(if n.forget_armed { 0 } else { 2 }, INK, 40)
                .inverted(n.forget_armed)
                .action(action::FORGET + i as ActionId),
            [p],
        );
        children.push(b);
    }
    let r = t.add(Node::row(Space::M), children);
    let r = t.add(Node::new(Kind::MinSize(Size::new(0, MIN_TARGET))), [r]);
    let r = t.pad(Insets::vh(Space::XS, Space::XS), r);
    let line = t.rule(1, RULE);
    let mut row = Node::column(0);
    if n.supported && !n.connected {
        row = row
            .action(action::NETWORK + i as ActionId)
            .feedback(Feedback::Outline);
    }
    t.add(row, [r, line])
}

impl WifiScreen {
    pub fn build(_text: &mut TextEngine, s: &Styles, v: &WifiView) -> (Tree, Self) {
        let mut t = Tree::new();
        let (status, busy) = status_line(&mut t, s, &v.status);
        t.set_hidden(busy, !v.busy);
        let back = icon_button(&mut t, Icon::ArrowLeft, IconSize::STANDARD, action::BACK);
        let title = t.label(tr(Msg::WifiTitle), s.display());
        let g = t.gap(Space::XS, 1);
        let header = t.add(Node::row(0), [back, g, title]);
        let header = t.pad(Insets::new(0, 0, 0, -26), header);

        // The switch and the state.
        let icon = t.icon(Icon::Wifi, IconSize::STANDARD, INK);
        let label = t.label(tr(Msg::WifiTitle), s.ui());
        let mut state = Paragraph::plain(v.state, s.small(), Align::Left);
        state.style.max_lines = Some(2);
        let state = t.text(state);
        let words = t.add(Node::column(2).flex(1), [label, state]);
        let switch = t.custom(Toggle { on: v.on });
        let r = t.add(Node::row(Space::M), [icon, words, switch]);
        let r = t.add(Node::new(Kind::MinSize(Size::new(0, MIN_TARGET))), [r]);
        let r = t.pad(Insets::vh(Space::S, 0), r);
        let toggle = t.add(
            Node::new(Kind::Padding(Insets::all(0)))
                .action(action::TOGGLE)
                .feedback(Feedback::Outline),
            [r],
        );

        // The policy, read-only: `aristotlectl net` changes it.
        let (value, hint) = match v.policy {
            Policy::Stock => (
                tr(Msg::WifiPolicyOff).to_string(),
                tr(Msg::WifiPolicyOffHint).to_string(),
            ),
            Policy::Scoped { hosts, no_probe } => {
                let mut hint = tr(Msg::WifiPolicyScopedHint).to_string();
                if !no_probe {
                    hint = format!("{hint} {}", tr(Msg::WifiProbeHint));
                }
                (trn(Plural::WifiPolicyScoped, hosts), hint)
            }
        };
        let picon = t.icon(Icon::Lock, IconSize::STANDARD, INK);
        let plabel = t.label(tr(Msg::WifiPolicy), s.ui());
        let pvalue = t.label(&value, s.ui_bold());
        let mut phint = Paragraph::plain(&hint, s.small(), Align::Left);
        phint.style.max_lines = Some(4);
        let phint = t.text(phint);
        let pwords = t.add(Node::column(2).flex(1), [plabel, pvalue, phint]);
        let policy = t.add(Node::row(Space::M).cross(Cross::Start), [picon, pwords]);
        let policy = t.pad(Insets::vh(Space::S, 0), policy);
        let line1 = t.rule(1, RULE);
        let line2 = t.rule(1, RULE);

        // The networks.
        let scan = text_button(&mut t, tr(Msg::WifiScan), s.ui_bold(), action::SCAN);
        t.set_hidden(scan, v.list == ListState::Off);
        let h = heading(&mut t, s, tr(Msg::WifiNetworks));
        let fill = t.spacer(1);
        let head = t.add(Node::row(0).cross(Cross::End), [h, fill, scan]);
        let items: Vec<NodeId> = match v.list {
            ListState::Off | ListState::Scanning => {
                let msg = if v.list == ListState::Off {
                    tr(Msg::WifiOffHint)
                } else {
                    tr(Msg::WifiScanning)
                };
                let p = t.text(Paragraph::plain(msg, s.ui(), Align::Left));
                vec![t.pad(Insets::vh(Space::M, 0), p)]
            }
            ListState::Networks if v.networks.is_empty() => {
                let p = t.text(Paragraph::plain(
                    tr(Msg::WifiNoneFound),
                    s.ui(),
                    Align::Left,
                ));
                vec![t.pad(Insets::vh(Space::M, 0), p)]
            }
            ListState::Networks => v
                .networks
                .iter()
                .enumerate()
                .map(|(i, n)| network_row(&mut t, s, i, n))
                .collect(),
        };
        let list = t.add(pages(0).flex(1), items);
        let prev = icon_button(
            &mut t,
            Icon::ChevronLeft,
            IconSize::STANDARD,
            action::PREV_PAGE,
        );
        let next = icon_button(
            &mut t,
            Icon::ChevronRight,
            IconSize::STANDARD,
            action::NEXT_PAGE,
        );
        let pager_label = t.text(Paragraph::plain("", s.small(), Align::Center));
        let pager_label = t.add(Node::column(0).flex(1), [pager_label]);
        let pager = t.add(Node::row(0), [prev, pager_label, next]);
        let col = t.add(
            Node::column(0),
            [
                status, header, toggle, line1, policy, line2, head, list, pager,
            ],
        );
        let root = t.pad(
            Insets::new(0, margin(), crate::geom::mm(1.5), margin()),
            col,
        );
        t.set_root(root);
        let ids = WifiScreen {
            list,
            pager_label: t.node(pager_label).children[0],
            prev: t.node(prev).children[0],
            next: t.node(next).children[0],
            busy,
            toggle,
        };
        (t, ids)
    }

    /// Updates the pager after layout.
    pub fn sync(&self, t: &mut Tree) {
        let (page, count) = t.page(self.list);
        t.set_text(self.pager_label, &page_of(page + 1, count.max(1)));
        t.set_icon_gray(self.prev, if page == 0 { RULE } else { INK });
        t.set_icon_gray(self.next, if page + 1 >= count { RULE } else { INK });
    }

    /// Turns the list pages; true if `id` was a pager action.
    pub fn handle(&self, t: &mut Tree, id: ActionId) -> bool {
        let (page, _) = t.page(self.list);
        match id {
            action::PREV_PAGE => t.set_page(self.list, page.saturating_sub(1)),
            action::NEXT_PAGE => t.set_page(self.list, page + 1),
            _ => return false,
        };
        true
    }
}

/// Networks for the preview and tests: names from Prus and Mickiewicz.
pub fn sample_networks() -> Vec<NetworkRow<'static>> {
    let n = |name, signal, secured, saved, connected, supported| NetworkRow {
        name,
        signal,
        secured,
        saved,
        connected,
        supported,
        forget_armed: false,
    };
    vec![
        n("Soplicowo", 4, true, true, true, true),
        n("Dom Wokulskiego", 3, true, false, false, true),
        n("Biblioteka", 5, false, false, false, true),
        n("Uczelnia", 5, true, false, false, false),
        n("Sklep Mincla", 2, true, true, false, true),
        n("Kawiarnia pod Messalką", 1, false, false, false, true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{SCREEN_HEIGHT, SCREEN_WIDTH};
    use crate::text::{FamilyId, Fonts};
    use crate::tree::Hit;
    use crate::ui::Ui;

    const STATUS: StatusLine<'static> = StatusLine {
        clock: "21:38",
        battery: Some(0.8),
        charging: false,
    };

    fn ui(v: &WifiView) -> (Ui, WifiScreen) {
        let mut text = TextEngine::new(Fonts::embedded());
        let (tree, ids) = WifiScreen::build(&mut text, &Styles::new(FamilyId(0)), v);
        let mut ui = Ui::new(tree, text, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
        ui.layout();
        ids.sync(&mut ui.tree);
        ui.render();
        (ui, ids)
    }

    fn targets(ui: &Ui) -> Vec<ActionId> {
        let mut out = Vec::new();
        for y in (0..SCREEN_HEIGHT).step_by(8) {
            for x in (0..SCREEN_WIDTH).step_by(40) {
                if let Hit::Target(n) = ui.tree.hit(crate::geom::Point { x, y }) {
                    let a = ui.tree.node(n).action.unwrap();
                    if !out.contains(&a) {
                        out.push(a);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn rows_act_by_kind() {
        let nets = sample_networks();
        let v = WifiView {
            status: STATUS,
            on: true,
            state: "Connected to Soplicowo · 192.168.1.23",
            busy: false,
            list: ListState::Networks,
            networks: &nets,
            policy: Policy::Stock,
        };
        let (ui, _) = ui(&v);
        let t = targets(&ui);
        // The connected network and the enterprise one are not tappable;
        // saved networks can be forgotten.
        assert!(!t.contains(&action::NETWORK));
        assert!(t.contains(&(action::NETWORK + 1)));
        assert!(t.contains(&(action::NETWORK + 2)));
        assert!(!t.contains(&(action::NETWORK + 3)));
        assert!(t.contains(&action::FORGET));
        assert!(!t.contains(&(action::FORGET + 1)));
        assert!(t.contains(&action::TOGGLE) && t.contains(&action::SCAN));
    }

    #[test]
    fn off_hides_scan_and_the_list() {
        let v = WifiView {
            status: STATUS,
            on: false,
            state: "Off",
            busy: false,
            list: ListState::Off,
            networks: &[],
            policy: Policy::Scoped {
                hosts: 2,
                no_probe: false,
            },
        };
        let (ui, _) = ui(&v);
        let t = targets(&ui);
        assert!(!t.contains(&action::SCAN));
        assert!(t.iter().all(|a| *a < action::NETWORK));
    }

    #[test]
    fn a_state_change_is_one_partial_update() {
        let nets = sample_networks();
        let mut v = WifiView {
            status: STATUS,
            on: true,
            state: "On, not connected",
            busy: true,
            list: ListState::Networks,
            networks: &nets,
            policy: Policy::Stock,
        };
        let (mut ui, _) = ui(&v);
        v.state = "Connected to Soplicowo";
        v.busy = false;
        let mut text = TextEngine::new(Fonts::embedded());
        let (tree, ids) = WifiScreen::build(&mut text, &Styles::new(FamilyId(0)), &v);
        ui.set_tree_with(tree, crate::tree::Refresh::Auto);
        ui.layout();
        ids.sync(&mut ui.tree);
        let d = ui.render();
        assert_eq!(d.len(), 1, "{d:?}");
        assert_ne!(d[0].waveform, crate::Waveform::GC16);
        assert!(d[0].rect.h < SCREEN_HEIGHT / 3, "{d:?}");
    }
}
