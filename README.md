# kryx — Kryonix Unified CLI

Status: PARTIAL · Real source of truth: [`src/`](./src/).

Source-canonical Rust CLI for the Kryonix distribution. Orchestrates
host identity, system switch/update via `nh`, the diskless node image
publish flow, and the local diagnostic/status dashboards. Acts as the
runtime **bypass carrier** for the `kryonix.security.cliLockdown`
module — the CLI cannot be fully blocked because it needs to invoke
`nix` and `nh` internally.

| Field | Value |
|---|---|
| Language / edition | Rust 2024 |
| Manifest | `Cargo.toml`, `Cargo.lock` (locked — must commit both together) |
| Build entry | `default.nix` → `rustPlatform.buildRustPackage` |
| Flake outputs | `packages.default`, `packages.kryx`, `devShells.default` |
| Default branch | `main` |

---

## Status do orquestrador (auditado em 2026-07-24)

Esta tabela é gerada a partir de leitura direta de `src/cli/mod.rs` e
`src/main.rs`. Os comandos marcados `READY` já compilam, recebem flags
e retornam o tipo esperado; `WIP` significa que existe, mas há pelo menos
uma flag ou subcomando com implementação mockada ou parcial.

| Comando | Subcomando | Handler (`src/services/`) | Status | Notas |
|---|---|---|---|---|
| `Switch [target]` | — | `modules::run_switch` | READY | Aplica `setpriv` quando euid=0, `nh os switch` via path absoluto |
| `Update [--force-sync]` | — | `update::run_update` | READY | Pull em `/etc/kryonix` + `/etc/kryonixos` + `nix flake update`. Após o fix do commit `6fb58a4`, descobre o binário real via `modules::discover_real_nix_dir()` |
| `Status` | — | `status::run_status` | READY | Lê `/etc/os-release`, identidade, zpool, Incus, Podman, geração NixOS, kernel, uptime |
| `Doctor [--json]` | — | `diagnostics::run_doctor` | READY | 10 categorias: identity, system, git, zfs, network, cpu, memory, disk, services, security |
| `Identity [--json]` | — | `identity::check_identity` | READY | Server-side checked via `kryx::services::identity::check_identity()` |
| `Deploy [--config-path] [--force] [--hostname]` | — | `deployment::run_deploy` | READY | Gateado por `env::check_is_live_iso()` (bloqueado se instalado, exceto com `--force`) |
| `FactoryReset [--preserve-home]` | — | `deployment::run_factory_reset` | READY | Reverte Live ISO install; `--preserve-home` mantém `/home` e subvolumes persistentes |
| `System` | `Report` | `telemetry::report_heartbeat` | READY (parcial) | Lê `/var/lib/kryonix/manifest.json`, gera payload JSON; **envia só via `println!`** — não entrega real, ver WIP |
| `Theme` | — | `theme::run_apply_theme` | READY | Substitui classes Tailwind hardcoded por classes semânticas `bg-apple-*` |
| `Node` | `List` | `node::run_node_command` | READY | Lê `dnsmasq.leases` em 3 paths canônicos |
| `Node` | `Publish` | `node::run_node_command` | READY | Build declarativo de imagem diskless via `nix build /etc/kryonixos#nixosConfigurations.node-client…` — após o fix, bypass de lockdown aplicado |
| `Node` | `Reload` | `node::run_node_command` | READY | `systemctl restart ipxe-http-server tftpd` |
| `Node` | `Reboot [<mac-or-ip>]` | `node::run_node_command` | WIP | Log: `Mock: Enviando sinal de reboot (não implementado).` |
| `Feature` | `List [--json]` | `feature::list_features` | READY | Lê `/etc/kryonix/features.json`. Override via env `KRYONIX_FEATURES_PATH` para testes |
| `Setup` | — | (stub) | WIP | Imprime `"Setup não implementado ainda."` |
| `Shell [args…]` | — | `passthrough::shell` | READY | Passthrough para `/run/current-system/sw/bin/nix shell` |
| `Search [args…]` | — | `passthrough::search` | READY | Passthrough para `/run/current-system/sw/bin/nh search` |
| `Clean [args…]` | — | `passthrough::clean` | READY | Passthrough para `nh clean` |
| `Build [args…]` | — | `passthrough::build` | READY | Passthrough para `nix build` |
| `Run [args…]` | — | `passthrough::run` | READY | Passthrough para `nix run` |
| `Develop [args…]` | — | `passthrough::develop` | READY | Passthrough para `nix develop` |
| `Repl [args…]` | — | `passthrough::repl` | READY | Passthrough para `nix repl` |
| `Fmt [args…]` | — | `passthrough::fmt` | READY | Passthrough para `nix fmt` |
| `Completion <shell>` | — | `clap_complete` inline | READY | Suporta `bash`, `zsh`, `fish`, `elvish`, `powershell` |

---

## Authorization model (extraído de `src/main.rs:14-56`)

O `kryx` filtra subcomandos com base no **role** retornado por
`kryx::services::identity::check_identity()`. Os roles suportados estão
em `kryx::domain::identity::Role`:

```
Core | Node | ThinkServer | Desktop
```

| Role | Visível | Subcomandos ocultos |
|---|---|---|
| `Desktop` | `Identity`, `Setup`, `Doctor`, `System`, `Theme`, `Node`, `Feature`, passthroughs, `Completion` | `Deploy`, `Node`, `Feature` (somente as variantes perigosas) |
| `Core` / `ThinkServer` | Tudo | — |
| Zombie (sem identity) | `Identity`, `Setup`, passthroughs, `Completion` | `Deploy`, `Node`, `Switch`, `FactoryReset`, `Doctor`, `System`, `Theme`, `Feature` |

Além disso, o **Identity Guard Block** (linhas 58-72) exige
`check_identity()` válida para: `Deploy`, `FactoryReset`,
`Node::Publish`. Sem identity, esses comandos **falham antes de tocar em
storage/network**.

> ⚠️ O ciclo `Setup` (que cria a identity) hoje é stub. Em sistemas novos
> o bootstrap precisa ser externo (instalador NixOS ou script
> ad-hoc). Task em aberto — ver "Pendências".

---

## Layout do repositório

```
.
├── Cargo.toml             # 7 deps: clap 4.6, clap_complete 4.5, colored 2,
│                          # ureq 2.9, serde, serde_json, chrono, tokio, libc
├── Cargo.lock             # locked — commitar junto com Cargo.toml
├── default.nix            # rustPlatform.buildRustPackage, pname="kryx"
├── flake.nix              # packages.{default,kryx} + devShells.default
├── src/
│   ├── main.rs            # Clap matcher + capability gates (261 linhas)
│   ├── lib.rs             # pub mod {domain, services}
│   ├── cli/mod.rs         # Cli, Commands, sub-enums (125 linhas)
│   ├── domain/
│   │   ├── mod.rs
│   │   ├── config.rs      # InstallPlanV2 (espelho do kryxd)
│   │   ├── identity.rs    # Role, HostIdentity
│   │   └── manifest.rs    # SystemManifest
│   └── services/
│       ├── mod.rs         # re-exports
│       ├── modules.rs     # run_switch, discover_real_nix_dir(pub)
│       ├── update.rs      # run_update
│       ├── status.rs      # run_status
│       ├── deployment.rs   # run_deploy, run_factory_reset
│       ├── diagnostics.rs # run_doctor + 11 checks
│       ├── identity.rs    # check_identity (re-export kryx lib)
│       ├── env.rs         # check_is_live_iso
│       ├── feature.rs     # list_features
│       ├── theme.rs       # run_apply_theme
│       ├── node.rs        # run_node_command (List/Publish/Reload/Reboot)
│       ├── passthrough.rs # shell/search/clean/build/run/develop/repl/fmt
│       ├── translator.rs  # generate_nix_config
│       ├── telemetry.rs   # report_heartbeat
│       ├── virt_engine.rs # incus_list/launch/stop
│       └── fallback.rs    # run_legacy_fallback (scripts/*.sh)
```

> O subcrate `kryx` (lib Rust) vive em `Cargo.toml` via path-dependency
> resolvido pelo `rustPlatform.buildRustPackage`. Não há `src/lib.rs`
> local; o crate é `services::` + `domain::` apenas.

---

## Workflow de lockdown (cli-lockdown bypass)

Documentado na skill canônica `devops:kryx-nix-lockdown-pitfalls`. O
resumo do que está **realmente implementado** em código:

### Bypass aplicado em `update.rs`, `node.rs::Publish`, `modules.rs`

```rust
// 1. Procurar o binário real em /nix/store (rejeita wrappers ~400 bytes).
let real_nix_dir = modules::discover_real_nix_dir().ok_or_else(|| {
    "Could not locate a real nix binary in /nix/store. \
     The Kryonix cli-lockdown may have removed it, \
     which would break `kryx <sub>`. ..."
})?;

// 2. Prepend no PATH do subprocesso (não precisa sudo -u).
cmd.env("PATH", format!("{}:{}", real_nix_dir, std::env::var("PATH")?));

// 3. Injetar HOME bypassando /root/.gitconfig (libgit2 erro 7).
cmd.env("HOME", format!("/home/{}", SUDO_USER.unwrap_or("rocha")));

// 4. Injetar GIT_CONFIG_GLOBAL/SYSTEM = /dev/null.
cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
   .env("GIT_CONFIG_SYSTEM", "/dev/null");
```

### Bypass aplicado em `passthrough.rs` (shell/build/run/etc.)

```rust
const NIX_PATH: &str = "/run/current-system/sw/bin/nix";
const NH_PATH:  &str = "/run/current-system/sw/bin/nh";

// path absoluto direto, sem prepend de PATH
Command::new(NIX_PATH).env("GIT_CONFIG_GLOBAL", "/dev/null")...
```

### `setpriv` em `modules.rs::run_switch`

Quando o kryx é invocado via `sudo` (euid=0), o `nh` 4.x recusa
explicitamente rodar como root. O `kryx` re-dropa privilégios com
`setpriv --reuid=<uid> --regid=<uid> --clear-groups` antes de
chamar `nh os switch --elevation-strategy /run/wrappers/bin/sudo`,
e o `nh` mesmo só escala de volta para o bootloader. Detalhes em
`src/services/modules.rs:122-145`.

### Nem todos os call-sites ainda usam o bypass

| Local | Estado atual | Comentário |
|---|---|---|
| `modules.rs::run_switch` | ✅ aplicado | referência canônica |
| `update.rs::run_update` | ✅ aplicado | commit `6fb58a4` |
| `node.rs::NodeAction::Publish` | ✅ aplicado | commit `6fb58a4` |
| `passthrough.rs` (todos) | ✅ path absoluto | não precisa PATH-append |
| `status.rs::read_*` | ✅ path absoluto | `/run/current-system/sw/bin/{sensors,ip,df,incus,podman,systemctl,zpool}` |
| `virt_engine.rs::{incus_list,launch,stop}` | ⚠ path simples | `Command::new("incus")` direto. Sem alvo de lockdown, OK em sistemas atuais; documentar se o lockdown for ampliado |
| `telemetry.rs::report_heartbeat` (`zpool`) | ⚠ path simples | mesmo caso, OK |

---

## Variáveis de ambiente (lidas / injetadas)

| Env var | Origem | Função |
|---|---|---|
| `PATH` | injetada (patch com `real_nix_dir:real`) | garantir que subprocessos achem o nix real |
| `HOME` | injetada (`/home/<SUDO_USER>`) | bypass do `/root/.gitconfig` (libgit2 code 7) |
| `GIT_CONFIG_GLOBAL=/dev/null` | injetada | desabilita libgit2 de ler config global |
| `GIT_CONFIG_SYSTEM=/dev/null` | injetada | mesma razão para system-level config |
| `GIT_CONFIG_NOSYSTEM=1` | injetada | backup defensivo (modules.rs/run_switch) |
| `USER` | lida (`check_system`) | descobrir `home-manager-<USER>.service` no `kryx doctor` |
| `SUDO_USER` | lida (`run_switch`, `run_update`, `Publish`) | resolver UID/GID para `setpriv` |
| `KRYONIX_FEATURES_PATH` | lida (`feature::list_features`) | override do path de `features.json` para teste/mock |
| `KRYONIX_AUTH_PASSWORD` | (citado em skill, não exercitado aqui) | ver `kryx` lib |

---

## Comandos proibidos sem autorização (do `kryonix-dev/AGENTS.md`)

- `git add .` / `git add -A`
- `git reset --hard` / `git clean -fdx`
- `git push --force` / `git branch -D`
- `nix flake update` (use `nix flake metadata --json` ou `kryx update`)
- `nixos-rebuild switch` (use `kryx switch` com a versão local)
- `reboot` / `poweroff`

---

## Build e desenvolvimento

```sh
# Nix build (release oficial)
nix build .#kryx
result/bin/kryx --version

# Cargo build (desenvolvimento local)
cargo build --release
./target/release/kryx --version

# Dev shell com cargo/rustfmt/clippy
nix develop
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings   # ver nota abaixo
cargo test --locked
```

> **Baseline de clippy (jul/2026)**: ~27 erros preexistentes
> (maioria em `services/theme.rs`, `services/diagnostics.rs`,
> `services/status.rs`) — todos `collapsible_if`, `map_or`,
> `unused imports`. Não são regressão deste repo. Recomenda-se um
> sweep de `cargo clippy --fix` em commit próprio antes de qualquer
> PR que mexa nesses arquivos.

---

## Workflow correto de `kryx switch` (não pular passos)

Em ordem estrita, copiado da skill canônica:

```sh
# 1. Editar código em ~/Proyectos/kryonix-dev/repos/kryx-cli
cd ~/Proyectos/kryonix-dev/repos/kryx-cli

# 2. Build release (necessário para aplicar a própria mudança)
cargo build --release

# 3. Commit + push (NUNCA `git add .`)
git add <arquivos específicos>
git -c commit.gpgSign=false commit -m "fix(kryx): ..."
git push origin HEAD:main

# 4. Atualiza lock do /etc/kryonixos para o novo SHA
#    (helper script em scripts/update_flake_lock.py na skill)
python3 scripts/update_flake_lock.py

# 5. Aplica o switch USANDO O BINÁRIO NOVO (target/release), não o do Nix antigo
cd /tmp
sudo /home/rocha/Proyectos/kryonix-dev/repos/kryx-cli/target/release/kryx switch

# 6. Validar
sudo /run/current-system/sw/bin/kryx doctor 2>&1 | tail -5
```

> Por que `target/release`? O `/run/current-system/sw/bin/kryx` é o
> binário antigo; rebuildar com código novo exige o `target/release`
> acabado de compilar. É o chicken-and-egg de cada rebuild.

---

## Skill canônica referenciada

- [`devops:kryx-nix-lockdown-pitfalls`](https://github.com/RAGton/kryx-cli)
  (carregável via skill). Documenta os 5 bugs conhecidos. Esta
  documentação é **mais nova** que a skill (inclui o fix dos Bobs 1/3
  em `update.rs`/`node.rs` aplicado em `6fb58a4`, ainda não refletido
  na skill). Recomendação: rodar `skill_manage action=patch` na
  skill após merge, em PR separado.

---

## Pendências explícitas

1. **`Node::Reboot`** — stub com log `Mock: Enviando sinal de reboot
   (não implementado).`. Substituir por wake-on-lan ou RPC real.
2. **`System::Report` (`telemetry::report_heartbeat`)** — gera payload
   JSON mas só imprime, não envia. Definir endpoint (Vault?
   Local Kafka?) ou marcar como deprecated.
3. **`Setup`** — stub literal. Hoje o bootstrap inicial da `identity`
   precisa ser externo. Documentar caminho manual ou implementar.
4. **Drift da skill `devops:kryx-nix-lockdown-pitfalls`** — 5 dias
   desatualizada relativamente ao estado pós-commit `6fb58a4`.
   Criar PR de manutenção da skill.
5. **27 erros de clippy preexistentes** (theme.rs,
   diagnostics.rs, status.rs) — sweep independente.
6. **Cargo.toml usa `libc = "0.2.189"`** sem `cargo update -p libc`
   correspondente em clones com lock mais antigo (skill Bug #5).
   Hoje o lock deste repo está alinhado, mas atenção em clones novos.
7. **`virt_engine.rs` e `telemetry.rs`** ainda usam paths simples.
   Se `cli-lockdown` for ampliado para `incus` ou `zpool`, esse
   fluxo vai precisar do mesmo tratamento.
8. **`rustfmt` drift preexistente em `main.rs:238`** —
   `use clap_complete::{generate, Shell}` deveria estar ordenada.
   Fora do escopo deste commit.

---

## Licença

Unfree (uso interno Kryonix). Constante em `default.nix`
`meta.license = licenses.unfree`.

---

## Validação

| Check | Resultado (commit `6fb58a4`) |
|---|---|
| `cargo build --release` | exit 0 em 6.92s |
| `cargo clippy -- -D warnings` | 27 errors antes, 27 depois (zero regressão) |
| `rustfmt --check src/services/{modules,update,node}.rs` | exit 0 |
| `nm target/release/kryx \| grep discover_real_nix_dir` | símbolo `T` presente (LTO não descartou) |
| `git diff --check` | clean |

Esta tabela é auditável sem abrir IDE: rode `cargo build --release && \
cargo clippy -- -D warnings 2>&1 | grep -c '^error:'` após o merge.

#tags: ai-agent documentation kryx-cli
