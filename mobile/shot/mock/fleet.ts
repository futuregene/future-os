/**
 * A phone paired with five desktops, for the home screen's merged recency page.
 *
 * The harness's ordinary demo is one machine — enough for every other scenario,
 * and the wrong shape for the one page whose job is "what did I last touch,
 * anywhere". The merged list exists in the product (`mergeSessions`, the
 * desktop picker's `All desktops`, per-row machine glyphs), so this is a
 * fixture, not a feature: five paired machines whose conversations interleave by
 * how recently each machine answered, two of them still working.
 *
 * `?fleet=1` swaps it in (see `RemoteContext.tsx`); every machine is `connected`,
 * because only a connected machine contributes rows (see `CatalogEntry.status`) —
 * an offline one would be listed in the picker and contribute nothing, which is
 * a different picture.
 *
 * Names, workspaces and titles come in the frame's language (`./language`),
 * including the machine labels: a machine's name is the user's own, and one drawn
 * in the wrong language reads as a mock.
 */

import { shotLanguage } from "./language";

const zh = shotLanguage === "zh";
const now = Date.now();
const minute = 60_000;

/** One paired machine, as the picker, the rows' glyphs and the catalogue read it. */
export interface FleetMachine {
  desktopId: string;
  pairId: string;
  /** The user's own name for the machine, in the frame's language. */
  name: string;
  /** A glyph id from `@future-os/peer-icons`. */
  icon: string;
}

/**
 * The five machines. The first is the one the phone is talking to (the open
 * machine, whose catalogue is the app's own); the rest answer as peers.
 *
 * Icons are all different on purpose: a merged row names its machine by glyph
 * alone, so two machines sharing one would be two rows a reader cannot tell
 * apart.
 */
export const fleetMachines: FleetMachine[] = [
  {
    desktopId: "desktop_lab",
    pairId: "pair_lab",
    name: zh ? "实验室的 MacBook Pro" : "Lab MacBook Pro",
    icon: "laptop",
  },
  {
    desktopId: "desktop_office",
    pairId: "pair_office",
    name: zh ? "办公室主机" : "Office Desktop",
    icon: "desktop",
  },
  {
    desktopId: "desktop_home",
    pairId: "pair_home",
    name: zh ? "家里工作站" : "Home Workstation",
    icon: "home",
  },
  {
    desktopId: "desktop_rack",
    pairId: "pair_rack",
    name: zh ? "楼下的服务器" : "Downstairs Server",
    icon: "server",
  },
  {
    desktopId: "desktop_cloud",
    pairId: "pair_cloud",
    name: zh ? "云端构建机" : "Cloud Build Machine",
    icon: "cloud",
  },
];

/** The active machine only, in the shape `RemoteProvider.desktops` holds. */
export const fleetDesktops = fleetMachines.map(machine => ({
  desktopId: machine.desktopId,
  pairId: machine.pairId,
  name: machine.name,
  icon: machine.icon,
}));

/**
 * Every machine's workspaces, by workspace id.
 *
 * Ids carry their machine (`ws_lab_future_os`) because a workspace id is the
 * machine's, not global: two checkouts of the same project on two machines are
 * two groups, and the merged list must not fold one shut with the other.
 * `name` is what the row prints under its title.
 */
const fleetWorkspaceRows = [
  {
    machine: "desktop_lab",
    id: "ws_lab_future_os",
    name: "future-os",
    path: "/Users/lixin/future-os",
    pinned: true,
  },
  { machine: "desktop_lab", id: "ws_lab_ui", name: "ui", path: "/Users/lixin/future-os/.worktrees/ui" },
  { machine: "desktop_office", id: "ws_office_future_os", name: "future-os", path: "/Users/mei/future-os" },
  { machine: "desktop_office", id: "ws_office_notes", name: "notes", path: "/Users/mei/notes" },
  { machine: "desktop_home", id: "ws_home_notes", name: "notes", path: "/Users/lixin/notes" },
  { machine: "desktop_home", id: "ws_home_papers", name: "papers", path: "/Users/lixin/papers" },
  { machine: "desktop_rack", id: "ws_rack_data", name: "data", path: "/srv/data" },
  { machine: "desktop_cloud", id: "ws_cloud_future_os", name: "future-os", path: "/srv/future-os" },
  { machine: "desktop_cloud", id: "ws_cloud_deploy", name: "deploy", path: "/srv/deploy" },
];

/** The workspaces of one machine, in the shape the list folds by. */
export function fleetWorkspaces(desktopId: string) {
  return fleetWorkspaceRows
    .filter(workspace => workspace.machine === desktopId)
    .map(({ machine: _machine, ...workspace }) => workspace);
}

/**
 * One conversation, as the host reports it: which machine, where it lives and
 * when it moved. No run state: every row on this page is settled (see the note
 * on the two newest rows).
 *
 * `lastMessageAt` is the recency page's only ordering clock, so the rows are
 * written as minutes-ago and the *machine* they belong to is what interleaves
 * them: five machines, five ages, one list.
 */
interface FleetRow {
  machine: string;
  id: string;
  workspaceId: string;
  title: string;
  /** Minutes since the last reply. */
  repliedAgo: number;
}

const fleetRows: FleetRow[] = zh
  ? [
    // These two used to be blocked on the reader's approval, which is a state
    // `SessionList` draws as a warning glyph on the row. A photo of "every
    // machine in one list" is about the merge, and an amber `!` on the newest
    // rows reads as "something is wrong" — so every row here is an ordinary
    // finished conversation, and the page's active stretch is empty on purpose.
    // (Nothing is `running` or `queued` either: those rows spin a loader, and a
    // loader is a different PNG every frame, which a capture meant to be a
    // baseline cannot be.)
    {
      machine: "desktop_cloud",
      id: "sess_fleet_coverage",
      workspaceId: "ws_cloud_future_os",
      title: "跑一遍全量覆盖率",
      repliedAgo: 0.5,
    },
    {
      machine: "desktop_office",
      id: "sess_fleet_pr_diff",
      workspaceId: "ws_office_future_os",
      title: "把这个 PR 的 diff 拉下来",
      repliedAgo: 2,
    },
    // Newest reply first. Every machine answers, and no two of the five sit next
    // to each other by accident.
    {
      machine: "desktop_lab",
      id: "sess_fleet_recency",
      workspaceId: "ws_lab_future_os",
      title: "截图里的手机端最近列表",
      repliedAgo: 4,
    },
    {
      machine: "desktop_home",
      id: "sess_fleet_weekly",
      workspaceId: "ws_home_notes",
      title: "核对周报里的几个数字",
      repliedAgo: 9,
    },
    {
      machine: "desktop_rack",
      id: "sess_fleet_split_log",
      workspaceId: "ws_rack_data",
      title: "把 40MB 的日志切成十份",
      repliedAgo: 16,
    },
    {
      machine: "desktop_cloud",
      id: "sess_fleet_nightly",
      workspaceId: "ws_cloud_future_os",
      title: "夜间构建为什么慢了 40 秒",
      repliedAgo: 24,
    },
    {
      machine: "desktop_office",
      id: "sess_fleet_review",
      workspaceId: "ws_office_future_os",
      title: "评审：一次改三处的 PR",
      repliedAgo: 38,
    },
    {
      machine: "desktop_lab",
      id: "sess_fleet_rail",
      workspaceId: "ws_lab_ui",
      title: "侧栏宽度再调一次",
      repliedAgo: 55,
    },
    {
      machine: "desktop_home",
      id: "sess_fleet_samples",
      workspaceId: "ws_home_papers",
      title: "把两篇的样本量对一下",
      repliedAgo: 85,
    },
    {
      machine: "desktop_rack",
      id: "sess_fleet_prune",
      workspaceId: "ws_rack_data",
      title: "清理三个月前的构建产物",
      repliedAgo: 130,
    },
    {
      machine: "desktop_cloud",
      id: "sess_fleet_canary",
      workspaceId: "ws_cloud_deploy",
      title: "灰度发布回滚演练",
      repliedAgo: 180,
    },
    {
      machine: "desktop_office",
      id: "sess_fleet_outline",
      workspaceId: "ws_office_notes",
      title: "写一版季度回顾的提纲",
      repliedAgo: 270,
    },
    {
      machine: "desktop_lab",
      id: "sess_fleet_keymap",
      workspaceId: "ws_lab_future_os",
      title: "键位审计：ctrl+g 与 ctrl+d",
      repliedAgo: 360,
    },
    {
      machine: "desktop_home",
      id: "sess_fleet_minutes",
      workspaceId: "ws_home_notes",
      title: "把会议纪要整理成待办",
      repliedAgo: 480,
    },
  ]
  : [
    {
      machine: "desktop_cloud",
      id: "sess_fleet_coverage",
      workspaceId: "ws_cloud_future_os",
      title: "Run the full coverage pass",
      repliedAgo: 0.5,
    },
    {
      machine: "desktop_office",
      id: "sess_fleet_pr_diff",
      workspaceId: "ws_office_future_os",
      title: "Pull the diff for this PR",
      repliedAgo: 2,
    },
    {
      machine: "desktop_lab",
      id: "sess_fleet_recency",
      workspaceId: "ws_lab_future_os",
      title: "The phone's recency page",
      repliedAgo: 4,
    },
    {
      machine: "desktop_home",
      id: "sess_fleet_weekly",
      workspaceId: "ws_home_notes",
      title: "Check the numbers in the weekly report",
      repliedAgo: 9,
    },
    {
      machine: "desktop_rack",
      id: "sess_fleet_split_log",
      workspaceId: "ws_rack_data",
      title: "Split the 40MB log into ten parts",
      repliedAgo: 16,
    },
    {
      machine: "desktop_cloud",
      id: "sess_fleet_nightly",
      workspaceId: "ws_cloud_future_os",
      title: "Why the nightly build got 40s slower",
      repliedAgo: 24,
    },
    {
      machine: "desktop_office",
      id: "sess_fleet_review",
      workspaceId: "ws_office_future_os",
      title: "Review: a PR that touches three places",
      repliedAgo: 38,
    },
    {
      machine: "desktop_lab",
      id: "sess_fleet_rail",
      workspaceId: "ws_lab_ui",
      title: "Adjust the rail width once more",
      repliedAgo: 55,
    },
    {
      machine: "desktop_home",
      id: "sess_fleet_samples",
      workspaceId: "ws_home_papers",
      title: "Compare the two sample sizes",
      repliedAgo: 85,
    },
    {
      machine: "desktop_rack",
      id: "sess_fleet_prune",
      workspaceId: "ws_rack_data",
      title: "Prune build artifacts older than three months",
      repliedAgo: 130,
    },
    {
      machine: "desktop_cloud",
      id: "sess_fleet_canary",
      workspaceId: "ws_cloud_deploy",
      title: "Rollback drill for the canary release",
      repliedAgo: 180,
    },
    {
      machine: "desktop_office",
      id: "sess_fleet_outline",
      workspaceId: "ws_office_notes",
      title: "Outline the quarterly review",
      repliedAgo: 270,
    },
    {
      machine: "desktop_lab",
      id: "sess_fleet_keymap",
      workspaceId: "ws_lab_future_os",
      title: "Keymap audit: ctrl+g and ctrl+d",
      repliedAgo: 360,
    },
    {
      machine: "desktop_home",
      id: "sess_fleet_minutes",
      workspaceId: "ws_home_notes",
      title: "Turn the meeting notes into to-dos",
      repliedAgo: 480,
    },
  ];

/**
 * One machine's conversations, in the shape `CatalogEntry.sessions` holds.
 *
 * `createdAt` is the session's own start. Nothing on this page reads it (the
 * active stretch, which ranked by it, is empty), so it is simply a plausible ten
 * minutes before the last reply rather than a second table to keep in step.
 */
function sessionsOf(desktopId: string) {
  return fleetRows
    .filter(row => row.machine === desktopId)
    .map((row) => {
      const workspace = fleetWorkspaceRows.find(entry => entry.id === row.workspaceId);
      return {
        sessionId: row.id,
        threadId: row.id.replace("sess_", "th_"),
        title: row.title,
        mode: "workspace" as const,
        workspaceId: row.workspaceId,
        workspaceName: workspace?.name ?? null,
        createdAt: now - (row.repliedAgo + 10) * minute,
        streaming: false,
        lastMessageAt: now - row.repliedAgo * minute,
      };
    });
}

/**
 * Every paired machine's catalogue, as the merged list reads them.
 *
 * All five are `connected`: the page drops a machine that is not answering, and
 * "four machines' conversations" would be a different picture from the one this
 * fixture exists for.
 */
export const fleetCatalogs = fleetMachines.map(machine => ({
  desktopId: machine.desktopId,
  label: machine.name,
  status: "connected" as const,
  sessions: sessionsOf(machine.desktopId),
}));

/** The open machine's own conversations, for the single-machine surfaces. */
export const fleetActiveSessions = fleetCatalogs[0]!.sessions;

/** The other four machines, in `RemoteControls["catalogs"].machines` shape. */
export const fleetOtherMachines = fleetMachines.slice(1).map(machine => ({
  desktopId: machine.desktopId,
  label: machine.name,
  icon: machine.icon,
  active: false,
  status: "connected" as const,
  control: "active" as const,
}));
