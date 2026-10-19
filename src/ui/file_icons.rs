//! Explorer file and folder icons: a glyph from `assets/file-icons.ttf` (Seti-UI,
//! Devicons and Material Design glyphs subset from Nerd Fonts Symbols, see
//! `tools/subset_file_icons.py`) plus the colour it is drawn in.
use eframe::egui::{Color32, FontFamily};
use std::path::Path;

pub(crate) const FAMILY: &str = "file-icons";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileIcon {
    pub glyph: char,
    pub color: Color32,
}

pub(crate) fn family() -> FontFamily {
    FontFamily::Name(FAMILY.into())
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const BLUE: Color32 = rgb(0x4f, 0x9f, 0xe0);
const LIGHT_BLUE: Color32 = rgb(0x5c, 0xc6, 0xe6);
const CYAN: Color32 = rgb(0x2f, 0xb8, 0xc4);
const GREEN: Color32 = rgb(0x7c, 0xbf, 0x5a);
const TEAL: Color32 = rgb(0x3f, 0xb3, 0x93);
const YELLOW: Color32 = rgb(0xe5, 0xc0, 0x4a);
const ORANGE: Color32 = rgb(0xe3, 0x8a, 0x3c);
const RED: Color32 = rgb(0xe0, 0x5a, 0x55);
const PINK: Color32 = rgb(0xe0, 0x6c, 0xa8);
const PURPLE: Color32 = rgb(0xa8, 0x7d, 0xe0);
const GREY: Color32 = rgb(0x9a, 0xa0, 0xa8);
const BROWN: Color32 = rgb(0xb0, 0x84, 0x5c);
const FOLDER: Color32 = rgb(0xd6, 0xa8, 0x4e);

const fn icon(glyph: u32, color: Color32) -> FileIcon {
    FileIcon {
        glyph: match char::from_u32(glyph) {
            Some(glyph) => glyph,
            None => '?',
        },
        color,
    }
}

// Seti-UI
const SETI_ASM: u32 = 0xe637;
const SETI_AUDIO: u32 = 0xe638;
const SETI_BABEL: u32 = 0xe639;
const SETI_BAZEL: u32 = 0xe63a;
const SETI_BICEP: u32 = 0xe63b;
const SETI_C: u32 = 0xe649;
const SETI_CLOJURE: u32 = 0xe642;
const SETI_COFFEE: u32 = 0xe61b;
const SETI_CONFIG: u32 = 0xe615;
const SETI_CPP: u32 = 0xe646;
const SETI_CSHARP: u32 = 0xe648;
const SETI_CSS: u32 = 0xe614;
const SETI_CSV: u32 = 0xe64a;
const SETI_CU: u32 = 0xe64b;
const SETI_D: u32 = 0xe651;
const SETI_DART: u32 = 0xe64c;
const SETI_DB: u32 = 0xe64d;
const SETI_DEFAULT: u32 = 0xe612;
const SETI_DOCKER: u32 = 0xe650;
const SETI_EDITORCONFIG: u32 = 0xe652;
const SETI_ELIXIR: u32 = 0xe62d;
const SETI_ELM: u32 = 0xe62c;
const SETI_ESLINT: u32 = 0xe655;
const SETI_ETHEREUM: u32 = 0xe656;
const SETI_FAVICON: u32 = 0xe623;
const SETI_FIREBASE: u32 = 0xe657;
const SETI_FONT: u32 = 0xe659;
const SETI_FSHARP: u32 = 0xe65a;
const SETI_GIT_IGNORE: u32 = 0xe65d;
const SETI_GITHUB: u32 = 0xe65b;
const SETI_GITLAB: u32 = 0xe65c;
const SETI_GO: u32 = 0xe627;
const SETI_GODOT: u32 = 0xe65f;
const SETI_GRADLE: u32 = 0xe660;
const SETI_GRAPHQL: u32 = 0xe662;
const SETI_HASKELL: u32 = 0xe61f;
const SETI_HAXE: u32 = 0xe666;
const SETI_HTML: u32 = 0xe60e;
const SETI_ILLUSTRATOR: u32 = 0xe669;
const SETI_IMAGE: u32 = 0xe60d;
const SETI_JAVA: u32 = 0xe66d;
const SETI_JAVASCRIPT: u32 = 0xe60c;
const SETI_JENKINS: u32 = 0xe66e;
const SETI_JINJA: u32 = 0xe66f;
const SETI_JULIA: u32 = 0xe624;
const SETI_KOTLIN: u32 = 0xe634;
const SETI_LESS: u32 = 0xe60b;
const SETI_LICENSE: u32 = 0xe60a;
const SETI_LIQUID: u32 = 0xe670;
const SETI_LOCK: u32 = 0xe672;
const SETI_LUA: u32 = 0xe620;
const SETI_MAKEFILE: u32 = 0xe673;
const SETI_MARKDOWN: u32 = 0xe609;
const SETI_MAVEN: u32 = 0xe674;
const SETI_MUSTACHE: u32 = 0xe60f;
const SETI_NIM: u32 = 0xe677;
const SETI_NOTEBOOK: u32 = 0xe678;
const SETI_NPM: u32 = 0xe616;
const SETI_OCAML: u32 = 0xe67a;
const SETI_PDF: u32 = 0xe67d;
const SETI_PERL: u32 = 0xe67e;
const SETI_PHOTOSHOP: u32 = 0xe67f;
const SETI_PHP: u32 = 0xe608;
const SETI_PLATFORMIO: u32 = 0xe682;
const SETI_POWERSHELL: u32 = 0xe683;
const SETI_PRISMA: u32 = 0xe684;
const SETI_PROLOG: u32 = 0xe685;
const SETI_PUG: u32 = 0xe686;
const SETI_PYTHON: u32 = 0xe606;
const SETI_R: u32 = 0xe68a;
const SETI_REACT: u32 = 0xe625;
const SETI_REASONML: u32 = 0xe687;
const SETI_RESCRIPT: u32 = 0xe688;
const SETI_ROLLUP: u32 = 0xe689;
const SETI_RUBY: u32 = 0xe605;
const SETI_RUST: u32 = 0xe68b;
const SETI_SASS: u32 = 0xe603;
const SETI_SBT: u32 = 0xe68d;
const SETI_SCALA: u32 = 0xe68e;
const SETI_SETTINGS: u32 = 0xe690;
const SETI_SHELL: u32 = 0xe691;
const SETI_SLIM: u32 = 0xe692;
const SETI_SMARTY: u32 = 0xe693;
const SETI_STYLELINT: u32 = 0xe695;
const SETI_STYLUS: u32 = 0xe600;
const SETI_SUBLIME: u32 = 0xe696;
const SETI_SVELTE: u32 = 0xe697;
const SETI_SVG: u32 = 0xe698;
const SETI_SWIFT: u32 = 0xe699;
const SETI_TERRAFORM: u32 = 0xe69a;
const SETI_TEX: u32 = 0xe69b;
const SETI_TEXT: u32 = 0xe64e;
const SETI_TODO: u32 = 0xe69c;
const SETI_TSCONFIG: u32 = 0xe69d;
const SETI_TWIG: u32 = 0xe61c;
const SETI_TYPESCRIPT: u32 = 0xe628;
const SETI_VALA: u32 = 0xe69e;
const SETI_VIDEO: u32 = 0xe69f;
const SETI_VUE: u32 = 0xe6a0;
const SETI_WASM: u32 = 0xe6a1;
const SETI_WEBPACK: u32 = 0xe6a3;
const SETI_WORD: u32 = 0xe6a5;
const SETI_XLS: u32 = 0xe6a6;
const SETI_XML: u32 = 0xe619;
const SETI_YARN: u32 = 0xe6a7;
const SETI_YML: u32 = 0xe6a8;
const SETI_ZIG: u32 = 0xe6a9;
const SETI_ZIP: u32 = 0xe6aa;
const SETI_ASTRO: u32 = 0xe6b3;
const SETI_TOML: u32 = 0xe6b2;
const SETI_PRETTIER: u32 = 0xe6b4;
const SETI_ADA: u32 = 0xe6b5;
const SETI_LISP: u32 = 0xe6b0;
const SETI_SCHEME: u32 = 0xe6b1;
const SETI_FENNEL: u32 = 0xe6af;
const SETI_VLANG: u32 = 0xe6ac;
const SETI_NEOVIM: u32 = 0xe6ae;
const SETI_VIM: u32 = 0xe62b;
const SETI_ORG: u32 = 0xe633;
const SETI_EMACS: u32 = 0xe632;
const SETI_CRYSTAL: u32 = 0xe62f;
const SETI_PURESCRIPT: u32 = 0xe630;
const SETI_PUPPET: u32 = 0xe631;
const SETI_GRAILS: u32 = 0xe661;
const SETI_HACK: u32 = 0xe663;
const SETI_HAML: u32 = 0xe664;
const SETI_EJS: u32 = 0xe618;
const SETI_NUNJUCKS: u32 = 0xe679;
const SETI_SALESFORCE: u32 = 0xe68c;
const SETI_SPRING: u32 = 0xe694;
const SETI_MDO: u32 = 0xe675;
const SETI_PROJECT: u32 = 0xe601;
const SETI_GULP: u32 = 0xe610;
const SETI_GRUNT: u32 = 0xe611;
const SETI_BOWER: u32 = 0xe61a;
const SETI_KARMA: u32 = 0xe622;
const SETI_HEROKU: u32 = 0xe607;
const SETI_IONIC: u32 = 0xe66b;
const SETI_WGT: u32 = 0xe6a4;
const SETI_WAT: u32 = 0xe6a2;
const SETI_CAKE: u32 = 0xe63e;
const SETI_COLDFUSION: u32 = 0xe645;
const SETI_ODATA: u32 = 0xe67b;
const SETI_PIPELINE: u32 = 0xe680;
const SETI_PLAN: u32 = 0xe681;
const SETI_HEX: u32 = 0xe667;
const SETI_ARGDOWN: u32 = 0xe636;
const SETI_BSL: u32 = 0xe63c;
const SETI_CODE_CLIMATE: u32 = 0xe643;
const SETI_LIVESCRIPT: u32 = 0xe671;
// Devicons
const DEV_GIT: u32 = 0xe702;
const DEV_NGINX: u32 = 0xe776;
const DEV_ANGULAR: u32 = 0xe753;
const DEV_APACHE: u32 = 0xe72b;
const DEV_ERLANG: u32 = 0xe7b1;
const DEV_GROOVY: u32 = 0xe775;
const DEV_TERMINAL: u32 = 0xe795;
const DEV_CMAKE: u32 = 0xe794;
const DEV_SQLITE: u32 = 0xe7c4;
const DEV_NODEJS: u32 = 0xe719;
const DEV_ANSIBLE: u32 = 0xe723;
const DEV_DENO: u32 = 0xe7c0;
const DEV_BUN: u32 = 0xe76f;
const DEV_DOTNET: u32 = 0xe77f;
const DEV_ARDUINO: u32 = 0xe733;
const DEV_AWK: u32 = 0xe741;
const DEV_BLENDER: u32 = 0xe766;
const DEV_REDIS: u32 = 0xe76d;
const DEV_POSTGRES: u32 = 0xe76e;
const DEV_MONGODB: u32 = 0xe7a4;
const DEV_MYSQL: u32 = 0xe704;
const DEV_UNITY: u32 = 0xe721;
const DEV_NEXT: u32 = 0xe718;
// Material Design
const MD_JSON: u32 = 0xf0626;
const MD_FILE: u32 = 0xf0214;
const MD_FOLDER: u32 = 0xf024b;
const MD_FOLDER_OPEN: u32 = 0xf0770;
const MD_FOLDER_IMAGE: u32 = 0xf024f;
const MD_FOLDER_DOWNLOAD: u32 = 0xf024d;
const MD_FOLDER_LOCK: u32 = 0xf0250;
const MD_FOLDER_ZIP: u32 = 0xf06eb;
const MD_FOLDER_TEXT: u32 = 0xf0c82;
const MD_FOLDER_COG: u32 = 0xf107d;
const MD_FOLDER_TABLE: u32 = 0xf12e3;
const MD_FOLDER_MUSIC: u32 = 0xf1359;
const MD_FOLDER_SEARCH: u32 = 0xf0968;
const MD_FOLDER_NETWORK: u32 = 0xf0870;
const MD_FOLDER_STAR: u32 = 0xf069d;
const MD_FOLDER_KEY: u32 = 0xf08ac;
const MD_FOLDER_MULTIPLE: u32 = 0xf0253;
const MD_FOLDER_PLAY: u32 = 0xf19fa;
const MD_FOLDER_WRENCH: u32 = 0xf19fc;
const MD_FOLDER_HOME: u32 = 0xf10b5;
const MD_FOLDER_INFO: u32 = 0xf10b7;
const MD_FOLDER_ACCOUNT: u32 = 0xf024c;
const MD_FOLDER_ARROW_UP: u32 = 0xf19f0;
// Seti/custom folder glyphs
const FOLDER_NPM: u32 = 0xe5fa;
const FOLDER_GIT: u32 = 0xe5fb;
const FOLDER_CONFIG: u32 = 0xe5fc;
const FOLDER_GITHUB: u32 = 0xe5fd;

/// The icon for a file, chosen from its exact name first (`Dockerfile`,
/// `Cargo.toml`, `.gitignore`...) and then from its extension.
pub(crate) fn for_file(path: &Path) -> FileIcon {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    by_name(&name)
        .or_else(|| by_compound_extension(&name))
        .or_else(|| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .and_then(|ext| by_extension(&ext.to_ascii_lowercase()))
        })
        .unwrap_or_else(|| icon(MD_FILE, GREY))
}

/// The icon for a directory, specialised for well-known names.
pub(crate) fn for_folder(path: &Path, open: bool) -> FileIcon {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (closed, color) = match name.as_str() {
        "node_modules" => (FOLDER_NPM, GREEN),
        ".git" | ".gitlab" | ".hg" | ".svn" => (FOLDER_GIT, ORANGE),
        ".github" | ".circleci" | ".buildkite" => (FOLDER_GITHUB, GREY),
        ".vscode" | ".idea" | ".zed" | ".lumen" | ".cargo" | ".config" | "config" | "configs"
        | "conf" | "settings" | ".husky" => (FOLDER_CONFIG, BLUE),
        "src" | "source" | "sources" | "lib" | "libs" | "crates" | "packages" | "app" | "apps"
        | "pkg" | "cmd" | "internal" => (MD_FOLDER, TEAL),
        "test" | "tests" | "spec" | "specs" | "__tests__" | "e2e" | "fixtures" | "testdata"
        | "benches" | "benchmarks" => (MD_FOLDER_PLAY, GREEN),
        "docs" | "doc" | "documentation" | "wiki" | "plans" | "notes" => (MD_FOLDER_TEXT, BLUE),
        "assets" | "images" | "img" | "icons" | "static" | "public" | "media" | "screenshots"
        | "pictures" | "resources" | "res" => (MD_FOLDER_IMAGE, PINK),
        "audio" | "sounds" | "music" => (MD_FOLDER_MUSIC, PINK),
        "build" | "dist" | "out" | "target" | "bin" | "obj" | "release" | "debug" | ".next"
        | ".nuxt" | ".output" | "coverage" | ".cache" | ".turbo" | "__pycache__" | ".venv"
        | "venv" | "env" | ".tox" | ".gradle" => (MD_FOLDER_ZIP, GREY),
        "scripts" | "tools" | "tool" | "xtask" | "script" | "utils" | "util" | "helpers" => {
            (MD_FOLDER_WRENCH, ORANGE)
        }
        "data" | "db" | "database" | "migrations" | "seeds" | "sql" | "schemas" | "schema" => {
            (MD_FOLDER_TABLE, YELLOW)
        }
        "vendor" | "third_party" | "third-party" | "external" | "deps" | "extern" => {
            (MD_FOLDER_DOWNLOAD, GREY)
        }
        "secrets" | "private" | ".ssh" | "keys" | "certs" | "cert" | "auth" | "security" => {
            (MD_FOLDER_LOCK, RED)
        }
        "api" | "server" | "network" | "net" | "services" | "service" | "routes" | "http" => {
            (MD_FOLDER_NETWORK, LIGHT_BLUE)
        }
        "examples" | "example" | "demo" | "demos" | "samples" | "sample" => {
            (MD_FOLDER_STAR, YELLOW)
        }
        "components" | "ui" | "views" | "pages" | "screens" | "widgets" | "layouts"
        | "templates" | "styles" | "css" | "themes" => (MD_FOLDER_MULTIPLE, PURPLE),
        "models" | "model" | "types" | "entities" | "domain" | "core" | "common" | "shared" => {
            (MD_FOLDER_KEY, CYAN)
        }
        "plugins" | "extensions" | "addons" | "modules" | "hooks" | "middleware" => {
            (MD_FOLDER_COG, PURPLE)
        }
        "home" | "user" | "users" | "profile" => (MD_FOLDER_HOME, BLUE),
        "log" | "logs" | "tmp" | "temp" | "var" => (MD_FOLDER_SEARCH, GREY),
        "i18n" | "locales" | "locale" | "lang" | "translations" => (MD_FOLDER_INFO, BLUE),
        "agents" | "agent" | "bots" => (MD_FOLDER_ACCOUNT, TEAL),
        "release-notes" | "changes" | "changelog" => (MD_FOLDER_ARROW_UP, GREEN),
        _ => (MD_FOLDER, FOLDER),
    };
    let glyph = if open && closed == MD_FOLDER {
        MD_FOLDER_OPEN
    } else {
        closed
    };
    icon(glyph, color)
}

fn by_name(name: &str) -> Option<FileIcon> {
    Some(match name {
        "dockerfile"
        | "containerfile"
        | ".dockerignore"
        | "docker-compose.yml"
        | "docker-compose.yaml"
        | "compose.yml"
        | "compose.yaml" => icon(SETI_DOCKER, BLUE),
        "makefile" | "gnumakefile" | "justfile" | "taskfile.yml" | "taskfile.yaml" | "rakefile" => {
            icon(SETI_MAKEFILE, ORANGE)
        }
        "cmakelists.txt" => icon(DEV_CMAKE, GREY),
        "cargo.toml"
        | "cargo.lock"
        | "rust-toolchain"
        | "rust-toolchain.toml"
        | "rustfmt.toml"
        | ".rustfmt.toml"
        | "clippy.toml"
        | ".clippy.toml"
        | "deny.toml" => icon(SETI_RUST, ORANGE),
        "package.json" | "package-lock.json" | ".npmrc" | ".npmignore" | ".nvmrc" => {
            icon(SETI_NPM, RED)
        }
        "yarn.lock" | ".yarnrc" | ".yarnrc.yml" => icon(SETI_YARN, BLUE),
        "pnpm-lock.yaml" | "pnpm-workspace.yaml" => icon(SETI_NPM, ORANGE),
        "bun.lockb" | "bun.lock" | "bunfig.toml" => icon(DEV_BUN, GREY),
        "deno.json" | "deno.jsonc" | "deno.lock" => icon(DEV_DENO, GREEN),
        "tsconfig.json" | "jsconfig.json" | "tsconfig.base.json" => icon(SETI_TSCONFIG, BLUE),
        "webpack.config.js" | "webpack.config.ts" => icon(SETI_WEBPACK, LIGHT_BLUE),
        "rollup.config.js" | "rollup.config.ts" | "rollup.config.mjs" => icon(SETI_ROLLUP, RED),
        "gulpfile.js" | "gulpfile.ts" => icon(SETI_GULP, RED),
        "gruntfile.js" => icon(SETI_GRUNT, ORANGE),
        "bower.json" | ".bowerrc" => icon(SETI_BOWER, ORANGE),
        "karma.conf.js" => icon(SETI_KARMA, GREEN),
        ".babelrc" | "babel.config.js" | "babel.config.json" | ".babelrc.js" => {
            icon(SETI_BABEL, YELLOW)
        }
        ".eslintrc" | ".eslintrc.js" | ".eslintrc.json" | ".eslintrc.yml" | ".eslintrc.cjs"
        | "eslint.config.js" | "eslint.config.mjs" | "eslint.config.ts" | ".eslintignore" => {
            icon(SETI_ESLINT, PURPLE)
        }
        ".prettierrc" | ".prettierrc.json" | ".prettierrc.yml" | ".prettierrc.js"
        | "prettier.config.js" | ".prettierignore" => icon(SETI_PRETTIER, LIGHT_BLUE),
        ".stylelintrc" | ".stylelintrc.json" | "stylelint.config.js" => icon(SETI_STYLELINT, GREY),
        ".editorconfig" => icon(SETI_EDITORCONFIG, GREY),
        ".gitignore"
        | ".gitattributes"
        | ".gitmodules"
        | ".gitkeep"
        | ".gitconfig"
        | ".git-blame-ignore-revs"
        | ".mailmap" => icon(SETI_GIT_IGNORE, ORANGE),
        "license" | "licence" | "license.md" | "license.txt" | "licence.md" | "copying"
        | "unlicense" | "notice" => icon(SETI_LICENSE, YELLOW),
        "readme" | "readme.md" | "readme.txt" | "readme.rst" => icon(SETI_MARKDOWN, LIGHT_BLUE),
        "changelog" | "changelog.md" | "history.md" | "changes.md" => icon(SETI_TODO, GREEN),
        "authors" | "contributors" | "codeowners" | "maintainers" => icon(SETI_TEXT, GREY),
        "agents.md" | "claude.md" | "gemini.md" => icon(SETI_MARKDOWN, PURPLE),
        "jenkinsfile" => icon(SETI_JENKINS, RED),
        "gemfile" | "gemfile.lock" | ".ruby-version" | "guardfile" | "podfile" | "fastfile" => {
            icon(SETI_RUBY, RED)
        }
        "build.gradle"
        | "settings.gradle"
        | "build.gradle.kts"
        | "settings.gradle.kts"
        | "gradle.properties"
        | "gradlew"
        | "gradlew.bat" => icon(SETI_GRADLE, TEAL),
        "pom.xml" => icon(SETI_MAVEN, RED),
        "build.sbt" => icon(SETI_SBT, RED),
        "build" | "workspace" | "build.bazel" | "workspace.bazel" | ".bazelrc"
        | ".bazelversion" => icon(SETI_BAZEL, GREEN),
        "platformio.ini" => icon(SETI_PLATFORMIO, ORANGE),
        "firebase.json" | ".firebaserc" => icon(SETI_FIREBASE, YELLOW),
        "vite.config.js" | "vite.config.ts" | "vite.config.mjs" | "vitest.config.ts" => {
            icon(SETI_JAVASCRIPT, PURPLE)
        }
        "next.config.js" | "next.config.mjs" | "next.config.ts" => icon(DEV_NEXT, GREY),
        "nuxt.config.js" | "nuxt.config.ts" => icon(DEV_NODEJS, GREEN),
        "angular.json" | ".angular-cli.json" => icon(DEV_ANGULAR, RED),
        "svelte.config.js" => icon(SETI_SVELTE, ORANGE),
        "astro.config.mjs" | "astro.config.ts" => icon(SETI_ASTRO, ORANGE),
        "tailwind.config.js" | "tailwind.config.ts" | "postcss.config.js" => icon(SETI_CSS, CYAN),
        "nginx.conf" => icon(DEV_NGINX, GREEN),
        ".htaccess" | "httpd.conf" => icon(DEV_APACHE, RED),
        "ansible.cfg" => icon(DEV_ANSIBLE, GREY),
        "procfile" => icon(SETI_HEROKU, PURPLE),
        "requirements.txt" | "pipfile" | "pipfile.lock" | "pyproject.toml" | "setup.py"
        | "setup.cfg" | "poetry.lock" | "tox.ini" | ".python-version" | "uv.lock" => {
            icon(SETI_PYTHON, YELLOW)
        }
        "go.mod" | "go.sum" | "go.work" => icon(SETI_GO, LIGHT_BLUE),
        "mix.exs" | "mix.lock" => icon(SETI_ELIXIR, PURPLE),
        "dune" | "dune-project" => icon(SETI_OCAML, ORANGE),
        "cabal.project" | "stack.yaml" => icon(SETI_HASKELL, PURPLE),
        "project.clj" | "deps.edn" => icon(SETI_CLOJURE, GREEN),
        "build.zig" | "build.zig.zon" => icon(SETI_ZIG, ORANGE),
        "package.swift" => icon(SETI_SWIFT, ORANGE),
        "pubspec.yaml" | "pubspec.lock" => icon(SETI_DART, BLUE),
        ".env" | ".envrc" | ".env.local" | ".env.example" | ".env.development"
        | ".env.production" | ".env.test" | ".env.sample" => icon(SETI_SETTINGS, YELLOW),
        ".bashrc" | ".bash_profile" | ".bash_login" | ".bash_logout" | ".zshrc" | ".zshenv"
        | ".zprofile" | ".profile" | ".zlogin" | ".zlogout" | ".inputrc" | ".fishrc" => {
            icon(SETI_SHELL, GREEN)
        }
        ".vimrc" | ".gvimrc" | "_vimrc" => icon(SETI_VIM, GREEN),
        ".ds_store" | "thumbs.db" | "desktop.ini" => icon(SETI_DEFAULT, GREY),
        "cname" | "robots.txt" | "humans.txt" => icon(SETI_TEXT, GREY),
        "favicon.ico" => icon(SETI_FAVICON, YELLOW),
        "godot.project" | "project.godot" => icon(SETI_GODOT, BLUE),
        "main.tf" | "variables.tf" | "outputs.tf" | "terraform.tfstate" | ".terraform.lock.hcl" => {
            icon(SETI_TERRAFORM, PURPLE)
        }
        "schema.prisma" => icon(SETI_PRISMA, LIGHT_BLUE),
        "vagrantfile" => icon(SETI_RUBY, BLUE),
        "brewfile" => icon(SETI_RUBY, ORANGE),
        _ => return None,
    })
}

fn by_compound_extension(name: &str) -> Option<FileIcon> {
    Some(match () {
        _ if name.ends_with(".d.ts") => icon(SETI_TYPESCRIPT, BLUE),
        _ if name.ends_with(".test.ts")
            || name.ends_with(".spec.ts")
            || name.ends_with(".test.tsx")
            || name.ends_with(".spec.tsx") =>
        {
            icon(SETI_TYPESCRIPT, GREEN)
        }
        _ if name.ends_with(".test.js")
            || name.ends_with(".spec.js")
            || name.ends_with(".test.jsx")
            || name.ends_with(".spec.jsx") =>
        {
            icon(SETI_JAVASCRIPT, GREEN)
        }
        _ if name.ends_with(".min.js") => icon(SETI_JAVASCRIPT, GREY),
        _ if name.ends_with(".min.css") => icon(SETI_CSS, GREY),
        _ if name.ends_with(".js.map") || name.ends_with(".css.map") || name.ends_with(".map") => {
            icon(SETI_DEFAULT, GREY)
        }
        _ if name.ends_with(".tar.gz")
            || name.ends_with(".tar.bz2")
            || name.ends_with(".tar.xz") =>
        {
            icon(SETI_ZIP, BROWN)
        }
        _ if name.starts_with(".env.") => icon(SETI_SETTINGS, YELLOW),
        _ if name.starts_with("dockerfile.") => icon(SETI_DOCKER, BLUE),
        _ if name.starts_with("docker-compose.") => icon(SETI_DOCKER, BLUE),
        _ if name.starts_with("readme.") => icon(SETI_MARKDOWN, LIGHT_BLUE),
        _ if name.starts_with("license.") => icon(SETI_LICENSE, YELLOW),
        _ => return None,
    })
}

fn by_extension(ext: &str) -> Option<FileIcon> {
    Some(match ext {
        // Systems
        "rs" => icon(SETI_RUST, ORANGE),
        "c" => icon(SETI_C, BLUE),
        "h" => icon(SETI_C, PURPLE),
        "ino" | "pde" => icon(DEV_ARDUINO, CYAN),
        "cpp" | "cc" | "cxx" | "c++" => icon(SETI_CPP, BLUE),
        "hpp" | "hh" | "hxx" | "h++" | "inl" | "ipp" | "tpp" => icon(SETI_CPP, PURPLE),
        "cu" | "cuh" => icon(SETI_CU, GREEN),
        "m" | "mm" => icon(SETI_C, LIGHT_BLUE),
        "go" => icon(SETI_GO, LIGHT_BLUE),
        "zig" | "zon" => icon(SETI_ZIG, ORANGE),
        "d" | "di" => icon(SETI_D, RED),
        "nim" | "nims" | "nimble" => icon(SETI_NIM, YELLOW),
        "v" | "vv" | "vsh" => icon(SETI_VLANG, BLUE),
        "vala" | "vapi" => icon(SETI_VALA, PURPLE),
        "cr" => icon(SETI_CRYSTAL, GREY),
        "ada" | "adb" | "ads" => icon(SETI_ADA, BLUE),
        "asm" | "s" | "nasm" | "masm" => icon(SETI_ASM, RED),
        "wasm" => icon(SETI_WASM, PURPLE),
        "wat" | "wast" => icon(SETI_WAT, PURPLE),
        // JVM / .NET
        "java" | "class" | "jar" | "war" => icon(SETI_JAVA, RED),
        "jsp" => icon(SETI_JAVA, ORANGE),
        "kt" | "kts" => icon(SETI_KOTLIN, ORANGE),
        "scala" | "sc" | "sbt" => icon(SETI_SCALA, RED),
        "groovy" | "gvy" | "gradle" => icon(DEV_GROOVY, TEAL),
        "clj" | "cljs" | "cljc" | "edn" => icon(SETI_CLOJURE, GREEN),
        "cs" | "csx" => icon(SETI_CSHARP, PURPLE),
        "csproj" | "sln" | "vbproj" | "fsproj" | "props" | "targets" => icon(DEV_DOTNET, PURPLE),
        "fs" | "fsx" | "fsi" => icon(SETI_FSHARP, BLUE),
        "vb" => icon(SETI_CSHARP, BLUE),
        "razor" | "cshtml" => icon(SETI_HTML, PURPLE),
        // Web
        "js" | "mjs" | "cjs" => icon(SETI_JAVASCRIPT, YELLOW),
        "jsx" => icon(SETI_REACT, LIGHT_BLUE),
        "ts" | "mts" | "cts" => icon(SETI_TYPESCRIPT, BLUE),
        "tsx" => icon(SETI_REACT, BLUE),
        "html" | "htm" | "xhtml" => icon(SETI_HTML, ORANGE),
        "css" => icon(SETI_CSS, BLUE),
        "scss" | "sass" => icon(SETI_SASS, PINK),
        "less" => icon(SETI_LESS, BLUE),
        "styl" | "stylus" => icon(SETI_STYLUS, GREEN),
        "vue" => icon(SETI_VUE, GREEN),
        "svelte" => icon(SETI_SVELTE, ORANGE),
        "astro" => icon(SETI_ASTRO, ORANGE),
        "coffee" | "litcoffee" => icon(SETI_COFFEE, BROWN),
        "ls" => icon(SETI_LIVESCRIPT, BLUE),
        "elm" => icon(SETI_ELM, LIGHT_BLUE),
        "re" | "rei" => icon(SETI_REASONML, RED),
        "res" | "resi" => icon(SETI_RESCRIPT, RED),
        "purs" => icon(SETI_PURESCRIPT, GREY),
        "pug" | "jade" => icon(SETI_PUG, BROWN),
        "hbs" | "handlebars" | "mustache" => icon(SETI_MUSTACHE, ORANGE),
        "ejs" => icon(SETI_EJS, YELLOW),
        "njk" | "nunjucks" => icon(SETI_NUNJUCKS, GREEN),
        "twig" => icon(SETI_TWIG, GREEN),
        "liquid" => icon(SETI_LIQUID, GREEN),
        "haml" => icon(SETI_HAML, ORANGE),
        "slim" => icon(SETI_SLIM, ORANGE),
        "j2" | "jinja" | "jinja2" => icon(SETI_JINJA, RED),
        "tpl" | "smarty" => icon(SETI_SMARTY, YELLOW),
        "wasm.map" => icon(SETI_WASM, GREY),
        "graphql" | "gql" => icon(SETI_GRAPHQL, PINK),
        "prisma" => icon(SETI_PRISMA, LIGHT_BLUE),
        "svg" => icon(SETI_SVG, YELLOW),
        // Scripting
        "py" | "pyw" | "pyi" | "pyx" | "pxd" | "pyc" | "pyo" => icon(SETI_PYTHON, YELLOW),
        "ipynb" => icon(SETI_NOTEBOOK, ORANGE),
        "rb" | "erb" | "gemspec" | "rake" | "ru" | "rbs" => icon(SETI_RUBY, RED),
        "php" | "phtml" | "php3" | "php4" | "php5" | "phps" => icon(SETI_PHP, PURPLE),
        "pl" | "pm" | "t" | "pod" => icon(SETI_PERL, BLUE),
        "lua" | "luau" => icon(SETI_LUA, BLUE),
        "r" | "rmd" | "rds" => icon(SETI_R, BLUE),
        "jl" => icon(SETI_JULIA, PURPLE),
        "swift" => icon(SETI_SWIFT, ORANGE),
        "dart" => icon(SETI_DART, CYAN),
        "ex" | "exs" | "eex" | "leex" | "heex" => icon(SETI_ELIXIR, PURPLE),
        "erl" | "hrl" | "beam" => icon(DEV_ERLANG, RED),
        "hs" | "lhs" | "cabal" => icon(SETI_HASKELL, PURPLE),
        "ml" | "mli" | "mll" | "mly" | "opam" => icon(SETI_OCAML, ORANGE),
        "lisp" | "lsp" | "cl" | "el" | "elc" => icon(SETI_LISP, GREY),
        "scm" | "ss" | "rkt" | "sld" => icon(SETI_SCHEME, RED),
        "fnl" => icon(SETI_FENNEL, GREEN),
        "pro" | "prolog" => icon(SETI_PROLOG, YELLOW),
        "awk" => icon(DEV_AWK, GREY),
        "hx" | "hxml" => icon(SETI_HAXE, ORANGE),
        "hack" | "hh.php" => icon(SETI_HACK, YELLOW),
        "gd" | "tscn" | "tres" | "godot" => icon(SETI_GODOT, BLUE),
        "unity" | "prefab" | "meta" => icon(DEV_UNITY, GREY),
        "blend" => icon(DEV_BLENDER, ORANGE),
        // Shell
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "tcsh" | "command" => {
            icon(SETI_SHELL, GREEN)
        }
        "ps1" | "psm1" | "psd1" => icon(SETI_POWERSHELL, BLUE),
        "bat" | "cmd" => icon(DEV_TERMINAL, GREEN),
        "vim" => icon(SETI_VIM, GREEN),
        "org" => icon(SETI_ORG, GREEN),
        // Data / config
        "json" | "jsonc" | "json5" | "jsonl" | "ndjson" | "geojson" | "webmanifest" => {
            icon(MD_JSON, YELLOW)
        }
        "yaml" | "yml" => icon(SETI_YML, PURPLE),
        "toml" => icon(SETI_TOML, ORANGE),
        "xml" | "xsd" | "xsl" | "xslt" | "plist" | "xaml" | "wsdl" | "rss" | "atom" | "dtd" => {
            icon(SETI_XML, ORANGE)
        }
        "ini" | "cfg" | "conf" | "config" | "properties" | "prefs" | "rc" | "desktop"
        | "service" => icon(SETI_CONFIG, GREY),
        "env" => icon(SETI_SETTINGS, YELLOW),
        "csv" | "tsv" => icon(SETI_CSV, GREEN),
        "sql" | "psql" | "mysql" | "pgsql" | "plsql" | "ddl" => icon(SETI_DB, LIGHT_BLUE),
        "sqlite" | "sqlite3" | "db" | "db3" | "mdb" | "accdb" => icon(DEV_SQLITE, LIGHT_BLUE),
        "mongo" | "bson" => icon(DEV_MONGODB, GREEN),
        "tf" | "tfvars" | "hcl" | "nomad" => icon(SETI_TERRAFORM, PURPLE),
        "bicep" => icon(SETI_BICEP, LIGHT_BLUE),
        "proto" | "thrift" | "avsc" | "fbs" | "capnp" => icon(SETI_ODATA, ORANGE),
        "cmake" => icon(DEV_CMAKE, GREY),
        "mk" | "mak" | "make" => icon(SETI_MAKEFILE, ORANGE),
        "bzl" | "bazel" => icon(SETI_BAZEL, GREEN),
        "lock" => icon(SETI_LOCK, GREY),
        "sol" => icon(SETI_ETHEREUM, GREY),
        "nix" => icon(SETI_CONFIG, LIGHT_BLUE),
        "dockerfile" => icon(SETI_DOCKER, BLUE),
        "pp" => icon(SETI_PUPPET, YELLOW),
        "yarnrc" => icon(SETI_YARN, BLUE),
        // Docs
        "md" | "markdown" | "mdown" | "mkd" | "mdx" => icon(SETI_MARKDOWN, LIGHT_BLUE),
        "rst" | "adoc" | "asciidoc" => icon(SETI_TEXT, LIGHT_BLUE),
        "txt" | "text" | "log" | "out" | "diff" | "patch" => icon(SETI_TEXT, GREY),
        "tex" | "latex" | "ltx" | "sty" | "cls" | "bib" => icon(SETI_TEX, TEAL),
        "pdf" => icon(SETI_PDF, RED),
        "doc" | "docx" | "dot" | "dotx" | "odt" | "rtf" | "pages" => icon(SETI_WORD, BLUE),
        "xls" | "xlsx" | "xlsm" | "ods" | "numbers" => icon(SETI_XLS, GREEN),
        "ppt" | "pptx" | "odp" | "key" => icon(MD_FILE, ORANGE),
        "todo" => icon(SETI_TODO, GREEN),
        "license" | "licence" => icon(SETI_LICENSE, YELLOW),
        // Media
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "avif" | "heic"
        | "heif" | "jfif" | "apng" | "tga" | "exr" | "hdr" | "dds" | "icns" | "raw" => {
            icon(SETI_IMAGE, PURPLE)
        }
        "ico" | "cur" => icon(SETI_FAVICON, YELLOW),
        "ai" | "eps" => icon(SETI_ILLUSTRATOR, ORANGE),
        "psd" | "psb" => icon(SETI_PHOTOSHOP, BLUE),
        "mp3" | "wav" | "flac" | "ogg" | "oga" | "m4a" | "aac" | "wma" | "aiff" | "opus"
        | "mid" | "midi" => icon(SETI_AUDIO, PINK),
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "m4v" | "mpg" | "mpeg" | "3gp"
        | "ogv" => icon(SETI_VIDEO, PINK),
        "ttf" | "otf" | "woff" | "woff2" | "eot" | "fon" => icon(SETI_FONT, RED),
        // Archives & binaries
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "lz" | "lzma"
        | "z" | "cab" | "iso" | "dmg" | "pkg" | "deb" | "rpm" | "apk" | "ipa" | "msi" | "crate"
        | "gem" | "whl" | "egg" => icon(SETI_ZIP, BROWN),
        "exe" | "dll" | "so" | "dylib" | "a" | "lib" | "o" | "obj" | "bin" | "elf" | "out.bin"
        | "wasm.gz" | "pdb" | "rlib" | "rmeta" | "node" | "ko" | "sys" | "app" => {
            icon(SETI_DEFAULT, GREY)
        }
        "hex" | "elf.hex" | "srec" | "ihex" => icon(SETI_HEX, GREY),
        // Security
        "pem" | "crt" | "cer" | "key.pem" | "p12" | "pfx" | "gpg" | "asc" | "pub" | "keystore"
        | "jks" => icon(SETI_LOCK, YELLOW),
        // IDE / project files
        "code-workspace" | "sublime-project" | "sublime-workspace" => icon(SETI_SUBLIME, ORANGE),
        "gitignore" | "gitattributes" | "gitmodules" | "gitkeep" => icon(SETI_GIT_IGNORE, ORANGE),
        "patch.orig" | "orig" | "rej" | "bak" | "swp" | "swo" | "tmp" => icon(SETI_DEFAULT, GREY),
        "git" => icon(DEV_GIT, ORANGE),
        "pipeline" => icon(SETI_PIPELINE, GREEN),
        "plan" => icon(SETI_PLAN, GREEN),
        "mdo" => icon(SETI_MDO, GREY),
        "wgt" => icon(SETI_WGT, GREY),
        "spring" => icon(SETI_SPRING, GREEN),
        "apex" | "cls.apex" | "trigger" => icon(SETI_SALESFORCE, LIGHT_BLUE),
        "arg" | "argdown" => icon(SETI_ARGDOWN, GREY),
        "bsl" => icon(SETI_BSL, GREY),
        "cfm" | "cfc" => icon(SETI_COLDFUSION, BLUE),
        "cake" => icon(SETI_CAKE, RED),
        "ionic" => icon(SETI_IONIC, BLUE),
        "heroku" => icon(SETI_HEROKU, PURPLE),
        "grails" => icon(SETI_GRAILS, GREEN),
        "mysql.sql" => icon(DEV_MYSQL, BLUE),
        "pg" => icon(DEV_POSTGRES, BLUE),
        "rdb" | "aof" => icon(DEV_REDIS, RED),
        "emacs" => icon(SETI_EMACS, PURPLE),
        "nvim" => icon(SETI_NEOVIM, GREEN),
        "project" => icon(SETI_PROJECT, BLUE),
        "codeclimate" => icon(SETI_CODE_CLIMATE, GREEN),
        "github" => icon(SETI_GITHUB, GREY),
        "gitlab" => icon(SETI_GITLAB, ORANGE),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(path: &str) -> u32 {
        for_file(Path::new(path)).glyph as u32
    }

    #[test]
    fn exact_names_beat_extensions() {
        assert_eq!(glyph("Cargo.toml"), SETI_RUST);
        assert_eq!(glyph("a/b/Dockerfile"), SETI_DOCKER);
        assert_eq!(glyph(".gitignore"), SETI_GIT_IGNORE);
        assert_eq!(glyph("Makefile"), SETI_MAKEFILE);
    }

    #[test]
    fn extensions_are_case_insensitive_and_compound_suffixes_win() {
        assert_eq!(glyph("Main.RS"), SETI_RUST);
        assert_eq!(glyph("index.d.ts"), SETI_TYPESCRIPT);
        assert_eq!(for_file(Path::new("a.spec.ts")).color, GREEN);
        assert_eq!(for_file(Path::new("a.ts")).color, BLUE);
    }

    #[test]
    fn unknown_files_get_the_generic_file_icon() {
        assert_eq!(glyph("whatever.zzzz"), MD_FILE);
    }

    #[test]
    fn folders_specialise_by_name_and_open_state() {
        assert_eq!(
            for_folder(Path::new("node_modules"), false).glyph as u32,
            FOLDER_NPM
        );
        assert_eq!(
            for_folder(Path::new("random"), false).glyph as u32,
            MD_FOLDER
        );
        assert_eq!(
            for_folder(Path::new("random"), true).glyph as u32,
            MD_FOLDER_OPEN
        );
    }

    #[test]
    fn every_glyph_used_is_in_the_bundled_font() {
        // All glyphs must resolve to real characters, never the '?' fallback.
        for ext in ["rs", "py", "json", "md", "png", "zip", "sh", "toml"] {
            let icon = by_extension(ext).unwrap();
            assert_ne!(icon.glyph, '?');
        }
    }
}
