mod diff_view;
mod file_tree;
mod viewer_options;

use crate::state::ViewerAppStateRef;
use eframe::egui;
use eframe::egui::Context;
use eframe::egui::panel::Side;

pub fn viewer_ui(ctx: &Context, state: &ViewerAppStateRef<'_>) {
    egui::SidePanel::new(Side::Left, "files").show(ctx, |ui| {
        file_tree::file_tree(ui, state);
    });

    egui::SidePanel::right("options").show(ctx, |ui| {
        ui.set_width(ui.available_width());

        viewer_options::viewer_options(ui, state);

        // // GitHub Authentication Section (WASM only)
        // #[cfg(target_arch = "wasm32")]
        // ui.group(|ui| {
        //     ui.heading("GitHub Integration");
        //
        //     if self.github_auth.is_authenticated() {
        //         if let Some(username) = self.github_auth.get_username() {
        //             ui.label(format!("✅ Signed in as {}", username));
        //         } else {
        //             ui.label("✅ Signed in");
        //         }
        //
        //         if ui.button("Sign Out").clicked() {
        //             self.github_auth.logout();
        //         }
        //     } else {
        //         ui.label("❌ Not signed in");
        //
        //         ui.separator();
        //         ui.heading("🔐 GitHub Authentication");
        //         ui.label("Sign in with GitHub to access private repositories and artifacts");
        //
        //         if ui.button("🚀 Sign in with GitHub").clicked() {
        //             self.github_auth.login_github();
        //         }
        //
        //         ui.separator();
        //         ui.label("💡 This uses Supabase for secure OAuth authentication");
        //         ui.label("Your GitHub token is safely managed and never exposed");
        //     }
        //
        //     ui.separator();
        //
        //     ui.label("GitHub Artifact URL:");
        //     ui.text_edit_singleline(&mut self.github_url_input);
        //
        //     if ui.button("Download Artifact").clicked() && !self.github_url_input.is_empty() {
        //         if let Some((owner, repo, artifact_id)) =
        //             parse_github_artifact_url(&self.github_url_input)
        //         {
        //             let api_url = github_artifact_api_url(&owner, &repo, &artifact_id);
        //             let token = self.github_auth.get_token().map(|t| t.to_string());
        //
        //             let source = DiffSource::Zip(PathOrBlob::Url(api_url, token));
        //
        //             // Clear existing snapshots
        //             self.snapshots.clear();
        //             self.index = 0;
        //             self.is_loading = true;
        //
        //             source.load(self.sender.clone(), ctx.clone(), self.settings.auth());
        //         } else {
        //             // Show error for invalid URL
        //             eprintln!("Invalid GitHub artifact URL");
        //         }
        //     }
        //
        //     if !self.github_url_input.is_empty()
        //         && parse_github_artifact_url(&self.github_url_input).is_none()
        //     {
        //         ui.colored_label(ui.visuals().error_fg_color, "Invalid GitHub artifact URL");
        //     }
        //
        //     ui.label("Expected format:");
        //     ui.monospace("github.com/owner/repo/actions/runs/12345/artifacts/67890");
        // });
        //
        // // GitHub PR Section
        // ui.group(|ui| {
        //     ui.heading("GitHub PR Integration");
        //
        //     ui.label("GitHub PR URL:");
        //     ui.text_edit_singleline(&mut self.github_pr_url_input);
        //
        //     ui.horizontal(|ui| {
        //         if ui.button("Load PR").clicked() && !self.github_pr_url_input.is_empty() {
        //             if let Ok((user, repo, pr_number)) =
        //                 parse_github_pr_url(&self.github_pr_url_input)
        //             {
        //                 let auth_token =
        //                     self.settings.auth().map(|auth| auth.provider_token.clone());
        //                 self.github_pr = Some(GithubPr::new(
        //                     user,
        //                     repo,
        //                     pr_number,
        //                     ctx.clone(),
        //                     auth_token,
        //                 ));
        //             } else {
        //                 eprintln!("Invalid GitHub PR URL");
        //             }
        //         }
        //
        //         if ui.button("Compare Branches Directly").clicked()
        //             && !self.github_pr_url_input.is_empty()
        //         {
        //             let source = DiffSource::Pr(self.github_pr_url_input.clone());
        //
        //             // Clear existing snapshots
        //             self.snapshots.clear();
        //             self.index = 0;
        //             self.is_loading = true;
        //
        //             source.load(self.sender.clone(), ctx.clone(), self.settings.auth());
        //         }
        //     });
        //
        //     if !self.github_pr_url_input.is_empty()
        //         && parse_github_pr_url(&self.github_pr_url_input).is_err()
        //     {
        //         ui.colored_label(ui.visuals().error_fg_color, "Invalid GitHub PR URL");
        //     }
        //
        //     ui.label("Expected format:");
        //     ui.monospace("https://github.com/owner/repo/pull/123");
        //
        //     // Show PR details and artifacts if available
        //     if let Some(pr) = &mut self.github_pr {
        //         ui.separator();
        //         if let Some(selected_source) = pr.ui(ui) {
        //             // Clear existing snapshots
        //             self.snapshots.clear();
        //             self.index = 0;
        //             self.is_loading = true;
        //
        //             selected_source.load(
        //                 self.sender.clone(),
        //                 ctx.clone(),
        //                 self.settings.auth(),
        //             );
        //         }
        //     }
        // });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        diff_view::diff_view(ui, state);
    });
}
