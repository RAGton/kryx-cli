# AGENTS.md — kryx-cli

> Regras operacionais para a CLI `kryx` (orquestrador Rust de comandos NixOS).

## Objective

Manter `kryx` como wrapper idiomático sobre comandos Nix nativos, com bypass do cli-lockdown via `discover_real_nix_dir` em `modules.rs`. Mudanças devem preservar a interface `<subcmd>` fina e serem cobertas por testes.

## Repository role

- CLI Rust (9 subcomandos: switch, update, rollback, gc, status, doctor, version, cap, lock)
- Single source of SSOT do lockdown bypass: `src/services/modules.rs::discover_real_nix_dir`
- Cli lockdown patch: aplicado em `update.rs` e `node.rs` pós-commit `6fb58a4`
- README: `README.md` (gerado de auditoria de código real)

## Versionamento e release

Este repo segue a **diretriz canônica unificada** do ecossistema Kryonix:

- **SSOT canônico:** [[kryonix-vault/02-Areas/Kryonix/canonical/release-process.md]]
- **Skill procedural:** `~/.hermes/skills/kryonix-versioning.md`
- **Manifesto:** `Cargo.toml` (linha `version`)
- **Tag prefix:** `v` (e.g., `v0.2.0`)
- **Lockdown bypass:** `vergen` para build-time SHA, ativar no `build.rs`

Antes de qualquer bump de versão, carregue a skill e siga o procedimento SSOT.

## Regras para agentes

- Não desenvolver diretamente em `/etc/kryonixos/`. Trabalhar em `repos/kryx-cli` no meta-repo.
- Cada commit = 1 escopo atômico. Mensagem: `kryx-cli: <descrição> (#$KANBAN_ID)`.
- Não commitar `vendor/` (ignorado via `.gitignore`).
- Não usar `git add .`, `git reset --hard`, `git push --force`.
- Path explícito em `git add`.
- `rustfmt` aceita alinhamento nativo — não lutar com o formatter.

## First steps before editing

1. Verificar que `target/release/kryx` tem o símbolo `discover_real_nix_dir` (`nm`).
2. Identificar o subcomando afetado (em `src/cli/`).
3. Procurar testes existentes (em `tests/`).
4. Preferir atualizar testes existentes a criar novos.

## Related notes

- Lockdown pitfalls canônico: [[kryonix-vault/02-Areas/Kryonix/canonical/kryx-nix-lockdown-pitfalls]]
- Release process SSOT: [[kryonix-vault/02-Areas/Kryonix/canonical/release-process]]