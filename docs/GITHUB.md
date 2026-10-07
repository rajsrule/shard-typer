# Upload Shard Typer to GitHub

The source upload folder contains the project files, including `.github`, `.cargo`, `.gitignore`, and `Cargo.lock`. It excludes `.git`, developer tools, build output, downloads, and saved text/settings. Upload its **contents**, so `Cargo.toml` is at the repository root.

## Upload using the website

1. Sign in and open your repository: [rajsrule/shard-typer](https://github.com/rajsrule/shard-typer).
2. Select **Add file → Upload files**.
3. Open the prepared source folder, select everything inside, and drag the selection onto the upload page. If you received a source ZIP, extract it first. Uploading the ZIP alone stores an archive rather than the project files.
4. Check that `src`, `assets`, `docs`, `scripts`, `packaging`, `examples`, `.cargo`, and `.github` appear as paths, alongside the root files. Include the files beginning with a dot. Do not drag the containing `ShardTyper-…-source` folder.
5. Enter **Add Shard Typer app**, choose to commit to `main`, and confirm **Commit changes**. An existing `LICENSE` can be replaced by the project's MIT license in the same commit.

GitHub accepts folders dragged onto the upload page, up to 100 files at once and 25 MiB per file. This project's source bundle fits those limits. See the [official upload instructions](https://docs.github.com/en/repositories/working-with-files/managing-files/adding-a-file-to-a-repository).

## Use GitHub Desktop for future updates

1. Install [GitHub Desktop](https://desktop.github.com/) and sign in.
2. Choose **File → Clone repository → URL**, enter your existing repository URL, and select a new local folder.
3. Copy the source upload folder's contents into the cloned folder. Keep the clone's `.git` folder.
4. Review the changed files, enter a commit summary, click **Commit to main**, then **Push origin**.
5. Use this cloned folder for subsequent code changes. Commit and push whenever you want to upload an update.

Cloning first preserves any initial license commit already on GitHub. It avoids combining two separate Git histories. See [cloning with GitHub Desktop](https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-and-forking-repositories-from-github-desktop) and [pushing changes](https://docs.github.com/en/desktop/contributing-and-collaborating-using-github-desktop/syncing-your-branch).

## Make the app downloadable

After uploading, open **Actions → Windows build**. Once the run succeeds, download its **ShardTyper-windows-x64** artifact; it contains the portable ZIP, installer, and checksums.

For a public release, open PowerShell in your cloned repository, create a version tag matching `Cargo.toml`, and push it:

```powershell
git tag v0.1.5
git push origin v0.1.5
```

The existing workflow builds the app and creates a **draft** release with the downloads attached. Open **Releases**, review the draft, and publish it. Create a new version/tag for each later release.
