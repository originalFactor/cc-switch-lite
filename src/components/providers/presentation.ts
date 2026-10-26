import type { TFunction } from "i18next";
import type { Provider } from "@/types";
import type { AppId } from "@/lib/api";
import type { AppMode } from "@/types/proxy";
import {
  isOfficialAccount,
  providerNeedsRouting,
  supportsFailover,
  supportsOfficialProxyTakeover,
} from "@/utils/providerCapabilities";

/**
 * 供应商卡片怎么画（v7）：当前那张的模式色、状态文字或按钮、徽标。由列表按应用和正在查看的
 * 模式算好交给卡片，卡片本身不再判断模式。
 */
export type CardTone = "direct" | "route" | "neutral";

export type ChipTone =
  "outline" | "direct" | "route" | "success" | "warning" | "danger";

export interface CardChip {
  key: string;
  label: string;
  tone: ChipTone;
  title?: string;
}

export interface CardMenuOption {
  key: string;
  label: string;
  detail?: string;
  onSelect: () => void;
}

export interface CardButton {
  key: string;
  label: string;
  onClick: () => void;
  /** 不能点的原因：按钮照常画出来，原因挂在按钮的说明卡上 */
  disabledReason?: string;
  /** 点开选一项 */
  menu?: { title: string; options: CardMenuOption[] };
}

export interface CardPresentation {
  /** 当前那张：模式色边框 + 淡底；neutral（灰底）只给共存式应用的默认那家 */
  tone?: CardTone;
  /** 主操作位换成状态文字（使用中 / 路由中 / 当前默认 / 已添加…） */
  status?: { label: string; dot: CardTone | "muted" };
  buttons: CardButton[];
  chips: CardChip[];
  /** 整卡淡一些（不能用于当前模式的官方订阅） */
  dim?: boolean;
  /** 故障转移队列里的上移 / 下移（键盘也能排序） */
  move?: { onUp?: () => void; onDown?: () => void };
  /** 在队列里且路由服务在跑时显示健康状态 */
  showHealth?: boolean;
  /** 删除不可用的原因（当前那家不能删…） */
  deleteDisabledReason?: string;
}

export interface ProviderSection {
  key: string;
  title?: string;
  help?: { title: string; body: string };
  emptyText?: string;
  items: { provider: Provider; presentation: CardPresentation }[];
}

/** 走路由时不能用的官方订阅（Codex 官方账号可以，靠客户端自己的登录）。 */
export function blockedFromRouting(app: AppId, provider: Provider): boolean {
  return (
    isOfficialAccount(app, provider) &&
    !supportsOfficialProxyTakeover(app, provider)
  );
}

// ─── 切换式应用（Claude Code / Codex / Gemini CLI / Grok Build）──────────────

export interface SwitchModeInput {
  app: AppId;
  t: TFunction;
  providers: Provider[];
  /** 现在生效的模式：`direct` / `route`（和页面头的开关一致）。 */
  active: AppMode;
  directId: string | null;
  routeId: string | null;
  failoverOn: boolean;
  /** 队列里的 id，按优先级 */
  queue: string[];
  /** 需要路由的原因（悬停「需要路由」时的说明） */
  routingReason: (provider: Provider) => string;
  serviceRunning: boolean;
  actions: {
    switchDirect: (provider: Provider) => void;
    needsRouteDialog: (provider: Provider) => void;
    /** 路由模式下换路由目标；已经在路由，不会再问一次。 */
    routeTo: (provider: Provider) => void;
    queueAdd: (provider: Provider) => void;
    queueRemove: (provider: Provider) => void;
    queueMove: (provider: Provider, delta: -1 | 1) => void;
  };
}

export function buildSwitchSections(input: SwitchModeInput): ProviderSection[] {
  const { t, active, directId, routeId } = input;
  // 后端 is_referenced：直连指针指着的、路由模式下正在路由的，都删不掉。
  const inUse = (id: string) =>
    id === directId || (active !== "direct" && id === routeId);
  const inUseReason = t("providerCard.reason.inUseCannotDelete");
  return buildSwitchSectionsByMode(input).map((section) => ({
    ...section,
    items: section.items.map((item) =>
      inUse(item.provider.id)
        ? {
            ...item,
            presentation: {
              ...item.presentation,
              deleteDisabledReason: inUseReason,
            },
          }
        : item,
    ),
  }));
}

function buildSwitchSectionsByMode(input: SwitchModeInput): ProviderSection[] {
  const {
    app,
    t,
    providers,
    active,
    directId,
    routeId,
    failoverOn,
    queue,
    actions,
  } = input;
  // 早期绑定托管账号的 Codex 官方卡没有 category，按身份认
  const official = (p: Provider) => isOfficialAccount(app, p);

  const chip = {
    official: (): CardChip => ({
      key: "official",
      label: t("providerCard.chip.official"),
      tone: "outline",
    }),
    needsRoute: (p: Provider): CardChip => ({
      key: "needsRoute",
      label: t("providerCard.chip.needsRoute"),
      tone: "outline",
      title: input.routingReason(p),
    }),
  };
  const base = (p: Provider) => {
    const chips: CardChip[] = [];
    if (official(p)) chips.push(chip.official());
    if (providerNeedsRouting(app, p)) chips.push(chip.needsRoute(p));
    return chips;
  };
  const officialOnly = (p: Provider) => (official(p) ? [chip.official()] : []);
  const blocked = (p: Provider, label: string, reason: string) => ({
    provider: p,
    presentation: {
      chips: officialOnly(p),
      dim: true,
      buttons: [
        {
          key: "blocked",
          label,
          onClick: () => undefined,
          disabledReason: reason,
        },
      ],
    } satisfies CardPresentation,
  });

  if (active === "direct") {
    // 直连：当前那家标「使用中」，其余是「切换」。需要路由的行打不开直连，先走确认框。
    return [
      {
        key: "all",
        items: providers.map((p) => {
          const chips = base(p);
          if (p.id === directId) {
            return {
              provider: p,
              presentation: {
                tone: "direct",
                status: {
                  label: t("providerCard.status.inUse"),
                  dot: "direct",
                },
                chips,
                buttons: [],
              },
            };
          }
          return {
            provider: p,
            presentation: {
              chips,
              buttons: [
                {
                  key: "switch",
                  label: t("providerCard.action.switch"),
                  onClick: () =>
                    providerNeedsRouting(app, p)
                      ? actions.needsRouteDialog(p)
                      : actions.switchDirect(p),
                },
              ],
            },
          };
        }),
      },
    ];
  }

  // active === "route"
  const noRoute = t("providerCard.reason.noRoute");
  if (!failoverOn) {
    return [
      {
        key: "all",
        items: providers.map((p) => {
          if (blockedFromRouting(app, p))
            return blocked(p, t("providerCard.action.routeHere"), noRoute);
          const chips = officialOnly(p);
          if (p.id === routeId) {
            return {
              provider: p,
              presentation: {
                tone: "route",
                status: {
                  label: t("providerCard.status.routing"),
                  dot: "route",
                },
                chips,
                buttons: [],
              },
            };
          }
          return {
            provider: p,
            presentation: {
              chips,
              buttons: [
                {
                  key: "routeHere",
                  label: t("providerCard.action.routeHere"),
                  onClick: () => actions.routeTo(p),
                },
              ],
            },
          };
        }),
      },
    ];
  }

  // 故障转移开着：队列 + 不在队列。P 序号和上下移按完整队列算，`providers`
  // 可能被搜索过滤过、只决定画哪些卡；「路由中」是后端记下的那家（转移成功后
  // 会换），不一定是队首
  const byId = new Map(providers.map((p) => [p.id, p]));
  const routing =
    routeId !== null && queue.includes(routeId) ? routeId : queue[0];
  const queueItems = queue.flatMap((id, index) => {
    const p = byId.get(id);
    if (!p) return [];
    const current = id === routing;
    return [
      {
        provider: p,
        presentation: {
          tone: current ? ("route" as const) : undefined,
          status: current
            ? {
                label: t("providerCard.status.routing"),
                dot: "route" as const,
              }
            : undefined,
          chips: [
            {
              key: "priority",
              label: `P${index + 1}`,
              tone: "outline" as const,
            },
          ],
          showHealth: input.serviceRunning,
          buttons: [
            {
              key: "queueRemove",
              label: t("providerCard.action.removeFromQueue"),
              onClick: () => actions.queueRemove(p),
            },
          ],
          move: {
            onUp: index > 0 ? () => actions.queueMove(p, -1) : undefined,
            onDown:
              index < queue.length - 1
                ? () => actions.queueMove(p, 1)
                : undefined,
          },
        } satisfies CardPresentation,
      },
    ];
  });
  const rest = providers
    .filter((p) => !queue.includes(p.id))
    .map((p) => {
      if (blockedFromRouting(app, p))
        return blocked(p, t("providerCard.action.addToQueue"), noRoute);
      // Codex 官方账号卡能路由，但后端不让它进队列（靠客户端自己的登录，不能和别家轮换）
      if (!supportsFailover(app, p))
        return blocked(
          p,
          t("providerCard.action.addToQueue"),
          t("providerCard.reason.noFailover"),
        );
      return {
        provider: p,
        presentation: {
          chips: officialOnly(p),
          buttons: [
            {
              key: "queueAdd",
              label: t("providerCard.action.addToQueue"),
              onClick: () => actions.queueAdd(p),
            },
          ],
        } satisfies CardPresentation,
      };
    });
  return [
    {
      key: "queue",
      title: t("providerCard.section.queue", { count: queueItems.length }),
      help: {
        title: t("providerCard.section.queueHelpTitle"),
        body: t("providerCard.section.queueHelp"),
      },
      emptyText: t("providerCard.section.queueEmpty"),
      items: queueItems,
    },
    {
      key: "rest",
      title: t("providerCard.section.notInQueue", { count: rest.length }),
      items: rest,
    },
  ];
}

// ─── Claude Desktop ─────────────────────────────────────────────────────────

export function buildDesktopSections({
  t,
  providers,
  currentId,
  onSwitch,
}: {
  t: TFunction;
  providers: Provider[];
  currentId: string;
  onSwitch: (provider: Provider) => void;
}): ProviderSection[] {
  return [
    {
      key: "all",
      items: providers.map((p) => {
        const mapping = providerNeedsRouting("claude-desktop", p);
        const chips: CardChip[] = [];
        if (isOfficialAccount("claude-desktop", p)) {
          chips.push({
            key: "official",
            label: t("providerCard.chip.official"),
            tone: "outline",
          });
        }
        const current = p.id === currentId;
        chips.push(
          mapping
            ? {
                key: "mapping",
                label: t("providerCard.chip.mapping"),
                tone: current ? "outline" : "route",
              }
            : {
                key: "direct",
                label: t("providerCard.chip.direct"),
                tone: current ? "outline" : "direct",
              },
        );
        if (current) {
          return {
            provider: p,
            presentation: {
              tone: mapping ? "route" : "direct",
              status: {
                label: t("providerCard.status.inUse"),
                dot: mapping ? "route" : "direct",
              },
              chips,
              buttons: [],
              deleteDisabledReason: t("providerCard.reason.inUseCannotDelete"),
            },
          };
        }
        return {
          provider: p,
          presentation: {
            chips,
            buttons: [
              {
                key: "switch",
                label: t("providerCard.action.switch"),
                onClick: () => onSwitch(p),
              },
            ],
          },
        };
      }),
    },
  ];
}
