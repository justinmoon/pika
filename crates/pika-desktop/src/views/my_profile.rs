use std::path::PathBuf;

use base64::Engine as _;
use iced::widget::{button, column, container, row, rule, text, text_input, Space};
use iced::{Alignment, Element, Fill, Task, Theme};
use pika_core::{AppAction, MyProfileState};

use crate::icons;
use crate::theme;
use crate::views::avatar::avatar_circle;

#[derive(Debug)]
pub struct State {
    about: String,
    name: String,
    confirm_logout: bool,
    /// Local file path for immediate preview of the picked image.
    pending_picture_path: Option<String>,
    /// Image data waiting to be uploaded when Save is clicked.
    pending_image: Option<PendingImage>,
    /// Dispatch UploadMyProfileImage once SaveMyProfile completes.
    upload_after_save: bool,
    /// True while the Blossom upload + kind-0 publish is in flight.
    uploading: bool,
}

#[derive(Debug, Clone)]
struct PendingImage {
    image_base64: String,
    mime_type: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    AboutChanged(String),
    CopyAppVersion,
    CopyNpub,
    LogoutClicked,
    LogoutConfirmed,
    LogoutCancelled,
    NameChanged(String),
    PickProfileImage,
    ProfileImagePicked(Vec<PathBuf>),
    Save,
}

pub enum Event {
    AppAction(AppAction),
    CopyNpub,
    CopyAppVersion,
    Logout,
}

impl State {
    pub fn new(my_profile_state: &MyProfileState) -> State {
        State {
            about: my_profile_state.about.clone(),
            name: my_profile_state.name.clone(),
            confirm_logout: false,
            pending_picture_path: None,
            pending_image: None,
            upload_after_save: false,
            uploading: false,
        }
    }

    pub fn update(&mut self, message: Message) -> (Option<Event>, Option<Task<Message>>) {
        match message {
            Message::AboutChanged(about) => {
                self.about = about;
            }
            Message::CopyAppVersion => return (Some(Event::CopyAppVersion), None),
            Message::CopyNpub => return (Some(Event::CopyNpub), None),
            Message::LogoutClicked => {
                self.confirm_logout = true;
            }
            Message::LogoutConfirmed => return (Some(Event::Logout), None),
            Message::LogoutCancelled => {
                self.confirm_logout = false;
            }
            Message::NameChanged(name) => {
                self.name = name;
            }
            Message::PickProfileImage => {
                let task = Task::perform(
                    async {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("Choose profile picture")
                            .add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif"])
                            .pick_file()
                            .await;
                        match handle {
                            Some(h) => vec![h.path().to_path_buf()],
                            None => vec![],
                        }
                    },
                    Message::ProfileImagePicked,
                );
                return (None, Some(task));
            }
            Message::ProfileImagePicked(paths) => {
                // Store image for upload on Save; show local preview now.
                if let Some(img) = prepare_profile_image(&paths) {
                    if let Some(path) = paths.first() {
                        self.pending_picture_path =
                            Some(format!("file://{}", path.to_string_lossy()));
                    }
                    self.pending_image = Some(img);
                }
            }
            Message::Save => {
                if self.pending_image.is_some() {
                    // Save name/about first; the deferred upload fires
                    // after the save completes (see take_deferred_upload).
                    self.upload_after_save = true;
                    self.uploading = true;
                }
                return (
                    Some(Event::AppAction(AppAction::SaveMyProfile {
                        name: self.name.clone(),
                        about: self.about.clone(),
                    })),
                    None,
                );
            }
        }

        (None, None)
    }

    /// Called by home when any state change is detected while the profile pane
    /// is open. Returns an upload action to dispatch if a deferred upload is
    /// ready (i.e. SaveMyProfile just completed).
    pub fn take_deferred_upload(&mut self) -> Option<AppAction> {
        if self.upload_after_save {
            self.upload_after_save = false;
            if let Some(img) = self.pending_image.take() {
                return Some(AppAction::UploadMyProfileImage {
                    image_base64: img.image_base64,
                    mime_type: img.mime_type,
                });
            }
        }
        None
    }

    /// Update drafts when the core profile state changes.
    pub fn sync_profile(&mut self, profile: &MyProfileState) {
        self.name = profile.name.clone();
        self.about = profile.about.clone();
        // Upload completed — backend now has the picture.
        if profile.picture_url.is_some() && self.pending_image.is_none() {
            self.pending_picture_path = None;
            self.uploading = false;
        }
    }

    pub fn view<'a>(
        &'a self,
        npub: &'a str,
        app_version: &'a str,
        picture_url: Option<&'a str>,
        avatar_cache: &mut super::avatar::AvatarCache,
    ) -> Element<'a, Message, Theme> {
        let mut content = column![].spacing(4).width(Fill);

        // ── Header ───────────────────────────────────────────────────
        content = content.push(
            container(
                text("Profile")
                    .size(16)
                    .font(icons::BOLD)
                    .color(theme::text_primary()),
            )
            .width(Fill)
            .center_x(Fill)
            .padding([16, 0]),
        );

        // ── Avatar (clickable to change) ─────────────────────────────
        let display_name = if self.name.is_empty() {
            "Me"
        } else {
            self.name.as_str()
        };
        let effective_picture = self.pending_picture_path.as_deref().or(picture_url);

        let avatar_label = if self.uploading {
            text("Uploading\u{2026}")
                .size(12)
                .color(theme::text_secondary())
        } else {
            text("Change photo").size(12).color(theme::accent_blue())
        };

        let avatar_button = button(
            column![
                avatar_circle(Some(display_name), effective_picture, 80.0, avatar_cache,),
                avatar_label,
            ]
            .spacing(4)
            .align_x(Alignment::Center),
        )
        .padding(4)
        .style(|_: &Theme, status: button::Status| {
            let bg = match status {
                button::Status::Hovered => theme::hover_bg(),
                _ => iced::Color::TRANSPARENT,
            };
            button::Style {
                background: Some(iced::Background::Color(bg)),
                border: iced::border::rounded(12),
                ..Default::default()
            }
        });

        let avatar_button = if self.uploading {
            avatar_button
        } else {
            avatar_button.on_press(Message::PickProfileImage)
        };

        content = content.push(
            container(avatar_button)
                .width(Fill)
                .center_x(Fill)
                .padding([8, 0]),
        );

        // ── Name field (icon + input row) ────────────────────────────
        content = content.push(icon_input_row(
            icons::USER,
            "Display name\u{2026}",
            self.name.as_str(),
            Message::NameChanged,
        ));

        // ── About field (icon + input row) ───────────────────────────
        content = content.push(icon_input_row(
            icons::PEN,
            "About\u{2026}",
            self.about.as_str(),
            Message::AboutChanged,
        ));

        // ── Save button ──────────────────────────────────────────────
        let save_button =
            button(text("Save Changes").size(14).font(icons::MEDIUM).center()).padding([10, 24]);

        let save_button = if self.uploading {
            save_button.style(theme::secondary_button_style)
        } else {
            save_button
                .on_press(Message::Save)
                .style(theme::primary_button_style)
        };

        content = content.push(
            container(save_button)
                .width(Fill)
                .center_x(Fill)
                .padding([8, 24]),
        );

        content = content.push(container(rule::horizontal(1)).padding([8, 24]));

        // ── npub row (icon + monospace + copy) ───────────────────────
        content = content.push(
            container(
                button(
                    row![
                        text(icons::KEY)
                            .font(icons::LUCIDE_FONT)
                            .size(18)
                            .color(theme::text_secondary()),
                        text(theme::truncated_npub_long(npub))
                            .size(14)
                            .font(icons::MONO)
                            .color(theme::text_secondary()),
                        Space::new().width(Fill),
                        text(icons::COPY)
                            .font(icons::LUCIDE_FONT)
                            .size(16)
                            .color(theme::text_faded()),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::CopyNpub)
                .width(Fill)
                .padding([12, 24])
                .style(ghost_row_style),
            )
            .width(Fill),
        );

        // ── Version row (icon + mono + copy) ─────────────────────────
        content = content.push(
            container(
                button(
                    row![
                        text(icons::INFO)
                            .font(icons::LUCIDE_FONT)
                            .size(18)
                            .color(theme::text_secondary()),
                        text(format!("Version {app_version}"))
                            .size(14)
                            .font(icons::MONO)
                            .color(theme::text_secondary()),
                        Space::new().width(Fill),
                        text(icons::COPY)
                            .font(icons::LUCIDE_FONT)
                            .size(16)
                            .color(theme::text_faded()),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::CopyAppVersion)
                .width(Fill)
                .padding([12, 24])
                .style(ghost_row_style),
            )
            .width(Fill),
        );

        content = content.push(container(rule::horizontal(1)).padding([8, 24]));

        // ── Logout ────────────────────────────────────────────────────
        content = content.push(Space::new().height(Fill));

        if self.confirm_logout {
            content = content.push(
                container(
                    row![
                        text("Log out?").size(14).color(theme::text_secondary()),
                        Space::new().width(Fill),
                        button(text("Cancel").size(13).center())
                            .on_press(Message::LogoutCancelled)
                            .padding([8, 16])
                            .style(theme::secondary_button_style),
                        button(text("Log out").size(13).center())
                            .on_press(Message::LogoutConfirmed)
                            .padding([8, 16])
                            .style(|_: &Theme, status: button::Status| {
                                let bg = match status {
                                    button::Status::Hovered => theme::danger().scale_alpha(0.85),
                                    _ => theme::danger(),
                                };
                                button::Style {
                                    background: Some(iced::Background::Color(bg)),
                                    text_color: iced::Color::WHITE,
                                    border: iced::border::rounded(8),
                                    ..Default::default()
                                }
                            }),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([12, 24]),
            );
        } else {
            content = content.push(
                button(
                    row![
                        text(icons::LOG_OUT)
                            .font(icons::LUCIDE_FONT)
                            .size(18)
                            .color(theme::danger()),
                        text("Logout").size(14).color(theme::danger()),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::LogoutClicked)
                .width(Fill)
                .padding([12, 24])
                .style(|_: &Theme, status: button::Status| {
                    let bg = match status {
                        button::Status::Hovered => theme::hover_bg(),
                        _ => iced::Color::TRANSPARENT,
                    };
                    button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: theme::danger(),
                        ..Default::default()
                    }
                }),
            );
        }

        container(content)
            .width(Fill)
            .height(Fill)
            .style(theme::surface_style)
            .into()
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn ghost_row_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => theme::hover_bg(),
        _ => iced::Color::TRANSPARENT,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::text_primary(),
        ..Default::default()
    }
}

/// Read an image file and prepare base64 + mime type for later upload.
fn prepare_profile_image(paths: &[PathBuf]) -> Option<PendingImage> {
    let path = paths.first()?;
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[pfp] failed to read {}: {e}", path.display());
            return None;
        }
    };
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mime_type = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
    .to_string();
    let image_base64 = base64::engine::general_purpose::STANDARD.encode(&data);
    Some(PendingImage {
        image_base64,
        mime_type,
    })
}

fn icon_input_row<'a>(
    icon_cp: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_input: impl 'a + Fn(String) -> Message,
) -> Element<'a, Message, Theme> {
    container(
        row![
            text(icon_cp)
                .font(icons::LUCIDE_FONT)
                .size(18)
                .color(theme::text_secondary()),
            text_input(placeholder, value)
                .on_input(on_input)
                .padding(10)
                .width(Fill)
                .style(theme::dark_input_style),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([4, 24])
    .into()
}
