use gpui_kit::component::{
    button::*,
    checkbox::Checkbox,
    dialog::DialogButtonProps,
    input::{Input, InputState, Textarea, TextareaState},
    resizable::{h_resizable, resizable_panel},
    text::TextView,
    *,
};
use gpui_kit::*;
use roc_request_bin::update::{UpdateManager, UpdateStatus};
use roc_request_bin::{server::Listener, store::*};
use std::{collections::HashSet, time::Duration};

// Keep captured content literal; it must never be interpreted as links or images.
fn literal(text: &str) -> String {
    let run = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((run + 1).max(3));
    format!("{fence}\n{text}\n{fence}")
}

pub struct Inbox {
    store: Store,
    settings: Settings,
    listener: Option<Listener>,
    rules: Vec<Rule>,
    requests: Vec<RequestSummary>,
    filter: Filter,
    total: usize,
    all_count: usize,
    unmatched_count: usize,
    page: usize,
    revision: u64,
    selected_rules: HashSet<i64>,
    selected_requests: HashSet<i64>,
    error: String,
    name: Entity<InputState>,
    prefix: Entity<InputState>,
    status_code: Entity<InputState>,
    content_type: Entity<InputState>,
    response_body: Entity<TextareaState>,
    editing_rule: Option<i64>,
    ip: Entity<InputState>,
    port: Entity<InputState>,
    methods: HashSet<String>,
    form_error: String,
    updates: UpdateManager,
    tray: Option<crate::tray::AppTray>,
}
impl Inbox {
    pub fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = store.settings().unwrap_or_default();
        let ip = cx.new(|cx| InputState::new(window, cx));
        ip.update(cx, |s, cx| s.set_value(settings.ip.clone(), window, cx));
        let port = cx.new(|cx| InputState::new(window, cx));
        port.update(cx, |s, cx| {
            s.set_value(settings.port.to_string(), window, cx)
        });
        let mut this = Self {
            store,
            settings,
            listener: None,
            rules: vec![],
            requests: vec![],
            filter: Filter::All,
            total: 0,
            all_count: 0,
            unmatched_count: 0,
            page: 0,
            revision: 0,
            selected_rules: HashSet::new(),
            selected_requests: HashSet::new(),
            error: String::new(),
            name: cx.new(|cx| InputState::new(window, cx).placeholder("例如：支付回调")),
            prefix: cx.new(|cx| InputState::new(window, cx).placeholder("/webhooks/")),
            status_code: cx.new(|cx| InputState::new(window, cx)),
            content_type: cx.new(|cx| InputState::new(window, cx)),
            response_body: cx.new(|cx| TextareaState::new(window, cx)),
            editing_rule: None,
            ip,
            port,
            methods: HashSet::from(["GET".into(), "POST".into()]),
            form_error: String::new(),
            updates: UpdateManager::new(),
            tray: None,
        };
        this.refresh();
        this.updates.check_if_due();
        let view = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            if let Some(view) = view.upgrade() {
                view.update(cx, |this, cx| match crate::tray::AppTray::new() {
                    Ok(tray) => {
                        this.tray = Some(tray);
                        window.on_window_should_close(cx, |window, cx| {
                            crate::tray::hide(window, cx);
                            false
                        });
                    }
                    Err(e) => {
                        this.error = format!("系统托盘不可用：{e}");
                        cx.notify();
                    }
                });
            }
        });
        cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                if view
                    .update_in(cx, |this, window, cx| {
                        this.updates.check_if_due();
                        let commands = this
                            .tray
                            .as_ref()
                            .map(|tray| tray.poll())
                            .unwrap_or_default();
                        for command in commands {
                            match command {
                                crate::tray::Command::Show => crate::tray::show(window, cx),
                                crate::tray::Command::Hide => crate::tray::hide(window, cx),
                                crate::tray::Command::Toggle => this.toggle_listener(cx),
                                crate::tray::Command::Quit => {
                                    this.listener.take();
                                    this.tray.take();
                                    cx.quit();
                                    return;
                                }
                            }
                        }
                        if let Some(tray) = this.tray.as_mut() {
                            tray.set_running(this.listener.is_some());
                        }
                        if this.store.revision() != this.revision {
                            this.refresh();
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }
    fn refresh(&mut self) {
        let revision = self.store.revision();
        let result = (|| -> anyhow::Result<()> {
            self.rules = self.store.rules()?;
            (self.total, self.requests) = self.store.page(self.filter, self.page)?;
            if self.page > 0 && self.requests.is_empty() {
                self.page = self.total.saturating_sub(1) / PAGE_SIZE;
                (self.total, self.requests) = self.store.page(self.filter, self.page)?;
            }
            (self.all_count, self.unmatched_count) = self.store.counts()?;
            self.revision = revision;
            Ok(())
        })();
        if let Err(e) = result {
            self.error = format!("数据库错误：{e}");
        }
    }
    fn choose(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.page = 0;
        self.selected_requests.clear();
        self.refresh();
        cx.notify();
    }
    fn toggle_server(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_listener(cx);
    }
    fn toggle_listener(&mut self, cx: &mut Context<Self>) {
        if self.listener.take().is_none() {
            let result = self
                .settings
                .ip
                .parse()
                .map_err(anyhow::Error::from)
                .and_then(|ip| {
                    Listener::start(
                        self.store.clone(),
                        std::net::SocketAddr::new(ip, self.settings.port),
                    )
                });
            match result {
                Ok(l) => {
                    self.listener = Some(l);
                    self.error.clear();
                }
                Err(e) => self.error = format!("启动失败：{e}"),
            }
        }
        cx.notify();
    }
    fn rule_dialog(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.open_rule_form(None, window, cx);
    }
    fn open_rule_form(&mut self, rule: Option<Rule>, window: &mut Window, cx: &mut Context<Self>) {
        self.form_error.clear();
        self.editing_rule = rule.as_ref().map(|r| r.id);
        let response = rule
            .as_ref()
            .map(|r| r.response.clone())
            .unwrap_or_default();
        self.name.update(cx, |s, cx| {
            s.set_value(
                rule.as_ref().map(|r| r.name.clone()).unwrap_or_default(),
                window,
                cx,
            )
        });
        self.prefix.update(cx, |s, cx| {
            s.set_value(
                rule.as_ref().map(|r| r.prefix.clone()).unwrap_or_default(),
                window,
                cx,
            )
        });
        self.methods = rule
            .as_ref()
            .map(|r| r.methods.iter().cloned().collect())
            .unwrap_or_else(|| HashSet::from(["GET".into(), "POST".into()]));
        self.status_code.update(cx, |s, cx| {
            s.set_value(response.status_code.to_string(), window, cx)
        });
        self.content_type
            .update(cx, |s, cx| s.set_value(response.content_type, window, cx));
        self.response_body
            .update(cx, |s, cx| s.set_value(response.response_body, window, cx));
        let view = cx.entity();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let this = view.read(cx);
            let mut checks = h_flex().flex_wrap().gap_3();
            for method in METHODS {
                let v = view.clone();
                let method = method.to_string();
                let m = method.clone();
                checks = checks.child(
                    Checkbox::new(SharedString::from(method.clone()))
                        .xsmall()
                        .label(method.clone())
                        .checked(this.methods.contains(&method))
                        .on_change(move |checked, window, cx| {
                            v.update(cx, |s, cx| {
                                if *checked {
                                    s.methods.insert(m.clone());
                                } else {
                                    s.methods.remove(&m);
                                }
                                cx.notify();
                            });
                            window.refresh();
                        }),
                );
            }
            let v = view.clone();
            dialog
                .title(if this.editing_rule.is_some() {
                    "编辑匹配规则"
                } else {
                    "新增匹配规则"
                })
                .w(px(560.))
                .margin_top(px(28.))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(if this.editing_rule.is_some() {
                            "保存规则"
                        } else {
                            "创建规则"
                        })
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .child(
                    v_flex()
                        .text_sm()
                        .gap_3()
                        .child("规则名称")
                        .child(Input::new(&this.name))
                        .child("路径前缀 · 忽略 scheme 和 host，区分大小写")
                        .child(Input::new(&this.prefix))
                        .child("HTTP Methods · 至少选择一项")
                        .child(checks)
                        .child(
                            h_flex()
                                .gap_3()
                                .child(
                                    v_flex()
                                        .w(px(130.))
                                        .gap_2()
                                        .child("Status code")
                                        .child(Input::new(&this.status_code)),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .gap_2()
                                        .child("Content-Type")
                                        .child(Input::new(&this.content_type)),
                                ),
                        )
                        .child("Response body")
                        .child(Textarea::new(&this.response_body).h(px(128.)).small())
                        .child(
                            div()
                                .text_color(rgb(0xc43c3c))
                                .child(this.form_error.clone()),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    window.refresh();
                    v.update(cx, |s, cx| {
                        let methods: Vec<_> = METHODS
                            .iter()
                            .filter(|m| s.methods.contains(**m))
                            .map(|m| m.to_string())
                            .collect();
                        let status_code = match s.status_code.read(cx).value().trim().parse::<u16>()
                        {
                            Ok(code) => code,
                            Err(_) => {
                                s.form_error = "Status code 必须为 200–599".into();
                                cx.notify();
                                return false;
                            }
                        };
                        let response = ResponseConfig {
                            status_code,
                            content_type: s.content_type.read(cx).value().trim().into(),
                            response_body: s.response_body.read(cx).value().into(),
                        };
                        match s.store.save_rule(
                            s.editing_rule,
                            s.name.read(cx).value().as_ref(),
                            s.prefix.read(cx).value().as_ref(),
                            &methods,
                            &response,
                        ) {
                            Ok(_) => {
                                s.page = 0;
                                s.selected_requests.clear();
                                s.refresh();
                                cx.notify();
                                true
                            }
                            Err(e) => {
                                s.form_error = e.to_string();
                                cx.notify();
                                false
                            }
                        }
                    })
                })
        });
    }
    fn settings_dialog(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.form_error.clear();
        self.ip.update(cx, |s, cx| {
            s.set_value(self.settings.ip.clone(), window, cx)
        });
        self.port.update(cx, |s, cx| {
            s.set_value(self.settings.port.to_string(), window, cx)
        });
        let view = cx.entity();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let this = view.read(cx);
            let v = view.clone();
            dialog
                .title("监听地址")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("保存")
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .child(
                    v_flex()
                        .gap_3()
                        .child("IP 地址（例如 127.0.0.1 或 ::1）")
                        .child(Input::new(&this.ip))
                        .child("HTTP 端口 · 1–65535")
                        .child(Input::new(&this.port))
                        .child("保存后停止当前监听，点击启动以使用新地址。")
                        .child(
                            div()
                                .text_color(rgb(0xc43c3c))
                                .child(this.form_error.clone()),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    window.refresh();
                    v.update(cx, |s, cx| {
                        let ip = s.ip.read(cx).value().trim().to_owned();
                        let port = s.port.read(cx).value().parse::<u16>();
                        if ip.parse::<std::net::IpAddr>().is_err() || !matches!(port, Ok(1..=65535))
                        {
                            s.form_error = "请输入有效的 IP 和端口（1–65535）".into();
                            cx.notify();
                            return false;
                        }
                        let settings = Settings {
                            ip,
                            port: port.unwrap(),
                            ..s.settings.clone()
                        };
                        match s.store.save_settings(&settings) {
                            Ok(()) => {
                                s.listener.take();
                                s.settings = settings;
                                s.error.clear();
                                cx.notify();
                                true
                            }
                            Err(e) => {
                                s.form_error = e.to_string();
                                cx.notify();
                                false
                            }
                        }
                    })
                })
        });
    }
    fn details(request: CapturedRequest, window: &mut Window, cx: &mut App) {
        window.open_sheet(cx, move |sheet, _, _| {
            let raw = request.export();
            let copy = raw.clone();
            let body = request.body_text();
            sheet
                .title(format!("{} · #{}", request.method, request.id))
                .size(px(620.))
                .child(
                    v_flex()
                        .size_full()
                        .gap_4()
                        .p_4()
                        .child(
                            Button::new("copy-all")
                                .primary()
                                .label("复制完整请求")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                }),
                        )
                        .child(
                            div()
                                .id("request-details")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .child(
                                    v_flex()
                                        .gap_4()
                                        .child(
                                            TextView::markdown(
                                                "request-url",
                                                literal(&format!(
                                                    "{}\n{}\n客户端 {}",
                                                    request.url,
                                                    format_args!(
                                                        "{} (UTC+8)",
                                                        display_time(&request.time)
                                                    ),
                                                    request.peer
                                                )),
                                            )
                                            .selectable(true),
                                        )
                                        .child(format!("请求头 · {}", request.headers.len()))
                                        .child(
                                            TextView::markdown(
                                                "request-headers",
                                                literal(
                                                    &request
                                                        .headers
                                                        .iter()
                                                        .map(|h| {
                                                            format!(
                                                                "{}: {}",
                                                                h.name,
                                                                h.display_value()
                                                            )
                                                        })
                                                        .collect::<Vec<_>>()
                                                        .join("\n"),
                                                ),
                                            )
                                            .selectable(true),
                                        )
                                        .child(format!("Body · {} bytes", request.body.len()))
                                        .child(
                                            TextView::markdown(
                                                "request-body",
                                                literal(if body.is_empty() {
                                                    "（无请求体）"
                                                } else {
                                                    &body
                                                }),
                                            )
                                            .selectable(true),
                                        ),
                                ),
                        ),
                )
        });
    }
    fn nav(
        &self,
        id: &'static str,
        label: String,
        filter: Filter,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .p_3()
            .rounded_md()
            .cursor_pointer()
            .bg(if self.filter == filter {
                rgb(0xe8f0fd)
            } else {
                rgb(0xf7f9fc)
            })
            .hover(|s| s.bg(rgb(0xeef3fa)))
            .child(label)
            .on_click(cx.listener(move |s, _, _, cx| s.choose(filter, cx)))
    }
}
impl Render for Inbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let update_status = self.updates.status();
        let width = f32::from(window.viewport_size().width);
        let mut sidebar = v_flex()
            .size_full()
            .bg(rgb(0xf7f9fc))
            .p_4()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("匹配规则"),
                    )
                    .child(
                        Button::new("add-rule")
                            .primary()
                            .label("＋ 新增")
                            .on_click(cx.listener(Self::rule_dialog)),
                    ),
            )
            .child(
                Button::new("delete-rules")
                    .label(format!("删除已选规则 ({})", self.selected_rules.len()))
                    .disabled(self.selected_rules.is_empty())
                    .on_click(cx.listener(|s, _, _, cx| {
                        let ids = s.selected_rules.iter().copied().collect::<Vec<_>>();
                        match s.store.delete_rules(&ids) {
                            Ok(()) => {
                                s.selected_rules.clear();
                                if matches!(s.filter,Filter::Rule(id) if ids.contains(&id)) {
                                    s.filter = Filter::All;
                                    s.page = 0;
                                    s.selected_requests.clear();
                                }
                                s.refresh();
                            }
                            Err(e) => s.error = e.to_string(),
                        }
                        cx.notify();
                    })),
            )
            .child(self.nav(
                "all",
                format!("全部请求    {}", self.all_count),
                Filter::All,
                cx,
            ))
            .child(self.nav(
                "unmatched",
                format!("未匹配    {}", self.unmatched_count),
                Filter::Unmatched,
                cx,
            ));
        let mut rule_list = v_flex().gap_2();
        for rule in &self.rules {
            let id = rule.id;
            let hint = format!(
                "{} · {} · {}",
                rule.name,
                rule.methods.join(", "),
                rule.prefix
            );
            rule_list = rule_list.child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(if self.filter == Filter::Rule(id) {
                        rgb(0xe8f0fd)
                    } else {
                        rgb(0xf7f9fc)
                    })
                    .child(
                        Checkbox::new(("rule-check", id as u64))
                            .checked(self.selected_rules.contains(&id))
                            .on_change(cx.listener(move |s, checked, _, cx| {
                                if *checked {
                                    s.selected_rules.insert(id);
                                } else {
                                    s.selected_rules.remove(&id);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .id(("rule", id as u64))
                            .flex_1()
                            .min_w_0()
                            .cursor_pointer()
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(hint.clone())
                                    .build(window, cx)
                            })
                            .gap_2()
                            .child(div().max_w(px(90.)).truncate().child(rule.name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .max_w(px(105.))
                                    .truncate()
                                    .text_color(rgb(0x53647c))
                                    .child(rule.methods.join(" · ")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .truncate()
                                    .child(rule.prefix.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .child(rule.count.to_string()),
                            )
                            .on_click(
                                cx.listener(move |s, _, _, cx| s.choose(Filter::Rule(id), cx)),
                            ),
                    )
                    .child(
                        Button::new(("edit-rule", id as u64))
                            .xsmall()
                            .icon(gpui_kit::assets::IconName::Pencil)
                            .tooltip("编辑规则")
                            .accessibility_label("编辑规则")
                            .on_click(cx.listener(move |s, _, window, cx| {
                                if let Some(rule) = s.rules.iter().find(|r| r.id == id).cloned() {
                                    s.open_rule_form(Some(rule), window, cx);
                                }
                            })),
                    ),
            );
        }
        sidebar = sidebar.child(
            div()
                .id("rules-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(rule_list),
        );
        let title = match self.filter {
            Filter::All => "全部请求".to_owned(),
            Filter::Unmatched => "未匹配".to_owned(),
            Filter::Rule(id) => self
                .rules
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.name.clone())
                .unwrap_or_default(),
        };
        let all_selected = !self.requests.is_empty()
            && self
                .requests
                .iter()
                .all(|r| self.selected_requests.contains(&r.id));
        let mut rows = v_flex();
        for (index, r) in self.requests.iter().enumerate() {
            let id = r.id;
            rows = rows.child(
                h_flex()
                    .w_full()
                    .min_h(px(58.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(0xe8edf3))
                    .hover(|s| s.bg(rgb(0xf4f7fc)))
                    .child(
                        Checkbox::new(("request-check", id as u64))
                            .checked(self.selected_requests.contains(&id))
                            .on_change(cx.listener(move |s, checked, _, cx| {
                                if *checked {
                                    s.selected_requests.insert(id);
                                } else {
                                    s.selected_requests.remove(&id);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .id(("request", id as u64))
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .cursor_pointer()
                            .child(
                                div()
                                    .w(px(44.))
                                    .text_color(rgb(0x53647c))
                                    .child((self.page * PAGE_SIZE + index + 1).to_string()),
                            )
                            .child(
                                div()
                                    .w(px(180.))
                                    .flex_shrink_0()
                                    .truncate()
                                    .text_xs()
                                    .child(display_time(&r.time)),
                            )
                            .child(
                                div()
                                    .w(px(70.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(if r.method == "GET" {
                                        0x137b69
                                    } else {
                                        0x315fd4
                                    }))
                                    .child(r.method.clone()),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(r.url.clone()))
                            .child(div().w(px(48.)).child(r.header_count.to_string()))
                            .child(div().w(px(56.)).child(if r.body_size == 0 {
                                "—"
                            } else {
                                "有"
                            }))
                            .on_click(cx.listener(
                                move |s, _, window, cx| match s.store.request(id) {
                                    Ok(request) => Self::details(request, window, cx),
                                    Err(e) => {
                                        s.error = e.to_string();
                                        cx.notify();
                                    }
                                },
                            )),
                    ),
            );
        }
        let mut content = v_flex()
            .flex_1()
            .min_h_0()
            .p_5()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        Button::new("delete-requests")
                            .label(format!("删除已选 ({})", self.selected_requests.len()))
                            .disabled(self.selected_requests.is_empty())
                            .on_click(cx.listener(|s, _, _, cx| {
                                match s.store.delete_requests(
                                    &s.selected_requests.iter().copied().collect::<Vec<_>>(),
                                ) {
                                    Ok(()) => {
                                        s.selected_requests.clear();
                                        s.refresh();
                                    }
                                    Err(e) => s.error = e.to_string(),
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .px_3()
                    .gap_3()
                    .text_xs()
                    .text_color(rgb(0x53647c))
                    .child(
                        Checkbox::new("select-page")
                            .checked(all_selected)
                            .on_change(cx.listener(|s, checked, _, cx| {
                                for r in &s.requests {
                                    if *checked {
                                        s.selected_requests.insert(r.id);
                                    } else {
                                        s.selected_requests.remove(&r.id);
                                    }
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().w(px(44.)).child("序号"))
                    .child(div().w(px(180.)).flex_shrink_0().child("请求时间"))
                    .child(div().w(px(70.)).child("METHOD"))
                    .child(div().flex_1().child("URL"))
                    .child(div().w(px(48.)).child("头数量"))
                    .child(div().w(px(56.)).child("BODY")),
            );
        if self.requests.is_empty() {
            content = content.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(div().text_xl().child("等待第一条请求"))
                    .child(
                        div()
                            .text_color(rgb(0x53647c))
                            .child("启动监听后，将 HTTP 请求发送到工具栏中的地址。"),
                    ),
            );
        } else {
            content = content.child(
                div()
                    .id("requests-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            );
        }
        content = content.child(
            h_flex()
                .justify_between()
                .child(format!(
                    "第 {} / {} 页",
                    self.page + 1,
                    self.total.div_ceil(PAGE_SIZE).max(1)
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("prev")
                                .label("上一页")
                                .disabled(self.page == 0)
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.page = s.page.saturating_sub(1);
                                    s.selected_requests.clear();
                                    s.refresh();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("next")
                                .label("下一页")
                                .disabled((self.page + 1) * PAGE_SIZE >= self.total)
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.page += 1;
                                    s.selected_requests.clear();
                                    s.refresh();
                                    cx.notify();
                                })),
                        ),
                ),
        );
        v_flex()
            .size_full()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x18283e))
            .text_sm()
            .child(
                h_flex()
                    .h(px(68.))
                    .px_5()
                    .gap_4()
                    .border_b_1()
                    .border_color(rgb(0xe2e8f0))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_lg()
                            .child("Roc Request Bin"),
                    )
                    .child(
                        Button::new("start-stop")
                            .primary()
                            .label(if self.listener.is_some() {
                                "停止监听"
                            } else {
                                "启动监听"
                            })
                            .on_click(cx.listener(Self::toggle_server)),
                    )
                    .child(
                        div()
                            .text_color(rgb(if self.listener.is_some() {
                                0x137b69
                            } else {
                                0x53647c
                            }))
                            .child(if self.listener.is_some() {
                                "● 监听中"
                            } else {
                                "○ 已停止"
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(0xc43c3c))
                            .child(self.error.clone()),
                    )
                    .child(
                        Button::new("address")
                            .label(format!(
                                "{}:{}",
                                if self.settings.ip.contains(':') {
                                    format!("[{}]", self.settings.ip)
                                } else {
                                    self.settings.ip.clone()
                                },
                                self.settings.port
                            ))
                            .on_click(cx.listener(Self::settings_dialog)),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("main-split")
                        .on_resize(cx.listener(
                            |s,
                             state: &Entity<gpui_kit::component::resizable::ResizableState>,
                             _,
                             cx| {
                                let sizes = state.read(cx).sizes();
                                if sizes.len() == 2 {
                                    let total = f32::from(sizes[0] + sizes[1]);
                                    if total > 0. {
                                        s.settings.sidebar_ratio =
                                            (f32::from(sizes[0]) / total).clamp(0.15, 0.65);
                                        if let Err(e) = s.store.save_settings(&s.settings) {
                                            s.error = e.to_string();
                                        }
                                    }
                                }
                            },
                        ))
                        .child(
                            resizable_panel()
                                .size(px(width * self.settings.sidebar_ratio))
                                .size_range(px(220.)..px(width * 0.65))
                                .child(sidebar),
                        )
                        .child(resizable_panel().child(content)),
                ),
            )
            .child(
                h_flex()
                    .h(px(32.))
                    .px_4()
                    .justify_between()
                    .border_t_1()
                    .border_color(rgb(0xe2e8f0))
                    .text_xs()
                    .text_color(rgb(0x53647c))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(update_status.message()),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Button::new("check-updates")
                                    .xsmall()
                                    .label(match &update_status {
                                        UpdateStatus::Checking => "检查中…",
                                        UpdateStatus::Installed(_) => "重启后生效",
                                        _ => "检查更新",
                                    })
                                    .disabled(!update_status.can_check())
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.updates.check_now();
                                        cx.notify();
                                    })),
                            )
                            .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                    ),
            )
    }
}
