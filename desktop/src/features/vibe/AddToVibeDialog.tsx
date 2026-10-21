import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { RemoteCatalog } from "../remote-peer/mergeConversations";
import type { VibeCell } from "./vibeCells";
import { Plus, X } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { IconButton } from "../../components/ui/IconButton";
import { Overlay } from "../../components/ui/Overlay";
import { TextInput } from "../../components/ui/TextInput";
import { cn } from "../../lib/cn";
import { vibeKeyOfRemote } from "./vibeCells";

/**
 * "Add a conversation to the panel": machine → workspace → conversation.
 *
 * The three columns are on screen at once rather than as a wizard. The user's own
 * requirement was to pick a machine and a workspace before picking a conversation,
 * and a remote controller has no patience for three screens of "next": here the
 * right-hand list answers the two choices on the left, and one press adds.
 *
 * **Adding is pinning**, and the dialog says so where it matters (the footer, once,
 * not on every row): the pin is what puts a conversation in the panel, so there is
 * no second membership list to drift from the rail's.
 */
export function AddToVibeDialog({
  catalogs,
  members,
  onAddLocal,
  onAddRemote,
  onClose,
  peerLabels,
  threads,
  workspaces,
}: {
  catalogs: readonly RemoteCatalog[];
  /** The conversations already in the panel, so the lists can leave them out. */
  members: readonly VibeCell[];
  onAddLocal: (thread: StoredThread) => void;
  onAddRemote: (desktopId: string, sessionId: string) => void;
  onClose: () => void;
  peerLabels: ReadonlyMap<string, string>;
  threads: readonly StoredThread[];
  workspaces: readonly StoredWorkspace[];
}) {
  const { t } = useTranslation("vibe");
  const machines = useMemo(
    () => [
      { id: "local", label: t("devices.local") },
      ...catalogs.map(catalog => ({ id: catalog.desktopId, label: peerLabels.get(catalog.desktopId) ?? catalog.desktopId })),
    ],
    [catalogs, peerLabels, t],
  );
  const [machine, setMachine] = useState(machines[0]?.id ?? "local");
  const [workspace, setWorkspace] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const inPanel = useMemo(() => new Set(members.map(cell => cell.key)), [members]);

  /** Workspaces offered for the chosen machine, as `{ id, name }` with chats under `null`. */
  const workspaceChoices = useMemo(() => {
    if (machine === "local") {
      return [
        { id: null as string | null, name: t("devices.chats") },
        ...workspaces
          .filter(item => item.kind === "user")
          .map(item => ({ id: item.id as string | null, name: item.name })),
      ];
    }
    const catalog = catalogs.find(item => item.desktopId === machine);
    const seen = new Map<string, string>();
    for (const session of catalog?.sessions ?? []) {
      if (session.workspaceId)
        seen.set(session.workspaceId, session.workspaceName ?? session.workspaceId);
    }
    return [
      { id: null as string | null, name: t("devices.chats") },
      ...[...seen].map(([id, name]) => ({ id, name })),
    ];
  }, [catalogs, machine, t, workspaces]);

  /**
   * The conversations that can be added: not pinned anywhere, on the chosen machine,
   * in the chosen workspace, matching the filter.
   *
   * Remote rows come from the host's catalogue, which is the only list this app has
   * of them — a host that is not connected contributes nothing, which is also why
   * adding from an offline machine is impossible rather than queued (`VIBE.md` §1).
   */
  const choices = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const match = (title: string) => !needle || title.toLowerCase().includes(needle);
    if (machine === "local") {
      return threads
        .filter(thread => thread.status === "active" && !thread.pinned)
        .filter(thread => (workspace === null ? thread.mode === "chat" : thread.workspaceId === workspace))
        .filter(thread => match(thread.title))
        .map(thread => ({ key: `local:${thread.id}`, title: thread.title, local: thread, remote: null }));
    }
    const catalog = catalogs.find(item => item.desktopId === machine);
    return (catalog?.sessions ?? [])
      .filter(session => session.pinned !== true)
      .filter(session => (workspace === null ? !session.workspaceId : session.workspaceId === workspace))
      .filter(session => match(session.title))
      .map(session => ({
        key: vibeKeyOfRemote(machine, session.sessionId),
        title: session.title,
        local: null,
        remote: session.sessionId,
      }));
  }, [catalogs, machine, query, threads, workspace]);

  function choose(next: string) {
    setMachine(next);
    // The workspace belongs to the machine, so a new machine starts from "chats"
    // rather than keeping an id that means nothing there.
    setWorkspace(null);
  }

  return (
    <Overlay onClose={onClose} open>
      <div
        aria-label={t("add.title")}
        className="relative flex h-[min(36rem,80vh)] w-[min(56rem,92vw)] flex-col overflow-hidden rounded-xl border border-line-soft bg-surface shadow-panel"
        role="dialog"
      >
        <header className="flex h-12 shrink-0 items-center gap-2 border-b border-line-soft px-4">
          <h2 className="min-w-0 flex-1 truncate text-sm font-semibold">{t("add.title")}</h2>
          <IconButton icon={<X className="size-4" />} label={t("add.close")} onClick={onClose} />
        </header>

        <div className="grid min-h-0 flex-1 grid-cols-[10rem_12rem_1fr]">
          {/* Machine */}
          <div className="min-h-0 overflow-y-auto border-r border-line-soft p-2">
            <div className="px-2 pb-1 pt-1 text-[11px] font-medium uppercase tracking-wide text-ink-faint">
              {t("add.machine")}
            </div>
            {machines.map(item => (
              <button
                className={cn(
                  "flex w-full items-center rounded-md px-2 py-1.5 text-left text-sm",
                  item.id === machine ? "bg-surface-subtle font-medium text-ink" : "text-ink-soft hover:bg-surface-subtle",
                )}
                data-add-machine={item.id}
                key={item.id}
                onClick={() => choose(item.id)}
                type="button"
              >
                <span className="truncate">{item.label}</span>
              </button>
            ))}
          </div>

          {/* Workspace */}
          <div className="min-h-0 overflow-y-auto border-r border-line-soft p-2">
            <div className="px-2 pb-1 pt-1 text-[11px] font-medium uppercase tracking-wide text-ink-faint">
              {t("add.workspace")}
            </div>
            {workspaceChoices.map(item => (
              <button
                className={cn(
                  "flex w-full items-center rounded-md px-2 py-1.5 text-left text-sm",
                  item.id === workspace ? "bg-surface-subtle font-medium text-ink" : "text-ink-soft hover:bg-surface-subtle",
                )}
                data-add-workspace={item.id ?? ""}
                key={item.id ?? "chats"}
                onClick={() => setWorkspace(item.id)}
                type="button"
              >
                <span className="truncate">{item.name}</span>
              </button>
            ))}
          </div>

          {/* Conversation */}
          <div className="flex min-h-0 flex-col p-2">
            <div className="px-1 pb-2 pt-1">
              <TextInput
                autoFocus
                className="h-8 w-full"
                onChange={event => setQuery(event.target.value)}
                placeholder={t("add.filter")}
                value={query}
              />
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {choices.length === 0
                ? <p className="px-2 py-3 text-xs text-ink-muted">{t("add.empty")}</p>
                : choices.map((choice) => {
                    const already = inPanel.has(choice.key);
                    return (
                      <div
                        className="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-surface-subtle"
                        data-add-choice={choice.key}
                        key={choice.key}
                      >
                        <span className="min-w-0 flex-1 truncate text-sm">{choice.title}</span>
                        <Button
                          disabled={already}
                          onClick={() => {
                            if (choice.local)
                              onAddLocal(choice.local);
                            else if (choice.remote)
                              onAddRemote(machine, choice.remote);
                          }}
                          size="xs"
                          variant={already ? "ghost" : "secondary"}
                        >
                          {already
                            ? t("add.added")
                            : (
                                <>
                                  <Plus className="size-3.5" />
                                  {t("add.add")}
                                </>
                              )}
                        </Button>
                      </div>
                    );
                  })}
            </div>
          </div>
        </div>

        <footer className="shrink-0 border-t border-line-soft px-4 py-2 text-xs text-ink-muted">
          {t("add.footer")}
        </footer>
      </div>
    </Overlay>
  );
}
