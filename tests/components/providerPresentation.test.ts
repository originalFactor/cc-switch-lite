import { describe, expect, it, vi } from "vitest";
import type { TFunction } from "i18next";
import type { Provider } from "@/types";
import type { AppMode } from "@/types/proxy";
import {
  buildDesktopSections,
  buildSwitchSections,
  type ProviderSection,
  type SwitchModeInput,
} from "@/components/providers/presentation";

const t = ((key: string, options?: Record<string, unknown>) =>
  options && "count" in options
    ? `${key}:${String(options.count)}`
    : key) as unknown as TFunction;

const provider = (id: string, overrides: Partial<Provider> = {}): Provider => ({
  id,
  name: id,
  settingsConfig: {},
  ...overrides,
});

const official = provider("official", { category: "official" });
const relay = provider("relay");
const backup = provider("backup");
/** Claude 下走 OpenAI 格式：需要路由 */
const converted = provider("converted", { meta: { apiFormat: "openai_chat" } });

function actions() {
  return {
    switchDirect: vi.fn(),
    needsRouteDialog: vi.fn(),
    routeTo: vi.fn(),
    queueAdd: vi.fn(),
    queueRemove: vi.fn(),
    queueMove: vi.fn(),
  };
}

function build(overrides: Partial<SwitchModeInput> & { active: AppMode }) {
  const input: SwitchModeInput = {
    app: "claude",
    t,
    providers: [official, relay, backup, converted],
    directId: "relay",
    routeId: null,
    failoverOn: false,
    queue: [],
    routingReason: () => "reason",
    serviceRunning: true,
    actions: actions(),
    ...overrides,
  };
  return { sections: buildSwitchSections(input), input };
}

const item = (sections: ProviderSection[], id: string) => {
  for (const section of sections) {
    const found = section.items.find((entry) => entry.provider.id === id);
    if (found) return { section: section.key, ...found.presentation };
  }
  throw new Error(`no card for ${id}`);
};

const button = (sections: ProviderSection[], id: string, key: string) => {
  const found = item(sections, id).buttons.find((b) => b.key === key);
  if (!found) throw new Error(`no ${key} button on ${id}`);
  return found;
};

describe("buildSwitchSections — direct", () => {
  it("marks the direct provider in use and asks before switching to one that needs routing", () => {
    const { sections, input } = build({ active: "direct" });

    expect(item(sections, "relay")).toMatchObject({
      tone: "direct",
      status: { label: "providerCard.status.inUse", dot: "direct" },
      buttons: [],
    });
    button(sections, "backup", "switch").onClick();
    expect(input.actions.switchDirect).toHaveBeenCalledWith(backup);

    button(sections, "converted", "switch").onClick();
    expect(input.actions.needsRouteDialog).toHaveBeenCalledWith(converted);
    expect(item(sections, "converted").chips.map((c) => c.key)).toContain(
      "needsRoute",
    );
  });

  it("keeps the whole list in one section without preview branches", () => {
    const { sections } = build({ active: "direct", routeId: "backup" });

    expect(sections.map((s) => [s.key, s.items.length])).toEqual([["all", 4]]);
    // 直连时上次的路由目标只是个记录，卡上不标、点它仍是直连切换
    expect(item(sections, "backup").status).toBeUndefined();
    expect(item(sections, "backup").chips).toEqual([]);
  });
});

describe("buildSwitchSections — delete guard", () => {
  // 后端 is_referenced：直连指针、路由模式下的当前路由都删不掉
  it("disables delete on the direct provider in both modes", () => {
    for (const active of ["direct", "route"] as AppMode[]) {
      const { sections } = build({ active, routeId: "backup" });
      expect(item(sections, "relay").deleteDisabledReason).toBe(
        "providerCard.reason.inUseCannotDelete",
      );
      expect(item(sections, "backup").deleteDisabledReason).toBe(
        active === "route"
          ? "providerCard.reason.inUseCannotDelete"
          : undefined,
      );
      expect(item(sections, "converted").deleteDisabledReason).toBeUndefined();
    }
  });

  it("does not count the remembered route target while direct", () => {
    const { sections } = build({ active: "direct", routeId: "backup" });
    expect(item(sections, "backup").deleteDisabledReason).toBeUndefined();
  });
});

describe("buildSwitchSections — route", () => {
  it("routes to a provider and keeps official subscriptions out", () => {
    const { sections, input } = build({ active: "route", routeId: "backup" });

    expect(item(sections, "backup")).toMatchObject({
      tone: "route",
      status: { label: "providerCard.status.routing" },
    });
    button(sections, "relay", "routeHere").onClick();
    expect(input.actions.routeTo).toHaveBeenCalledWith(relay);
    expect(item(sections, "relay").chips).toEqual([]);
    expect(item(sections, "official").dim).toBe(true);
    expect(button(sections, "official", "blocked").disabledReason).toBe(
      "providerCard.reason.noRoute",
    );
  });

  it("splits the failover queue from the rest and only moves within the queue", () => {
    const { sections, input } = build({
      active: "route",
      routeId: "relay",
      failoverOn: true,
      queue: ["relay", "backup"],
    });

    expect(sections.map((s) => [s.key, s.items.length])).toEqual([
      ["queue", 2],
      ["rest", 2],
    ]);
    expect(item(sections, "relay")).toMatchObject({
      tone: "route",
      showHealth: true,
      chips: [{ key: "priority", label: "P1" }],
    });
    expect(item(sections, "relay").move?.onUp).toBeUndefined();
    item(sections, "relay").move?.onDown?.();
    expect(input.actions.queueMove).toHaveBeenCalledWith(relay, 1);
    expect(item(sections, "backup").move?.onDown).toBeUndefined();

    button(sections, "converted", "queueAdd").onClick();
    expect(input.actions.queueAdd).toHaveBeenCalledWith(converted);
    button(sections, "backup", "queueRemove").onClick();
    expect(input.actions.queueRemove).toHaveBeenCalledWith(backup);

    // 直连那家不在队列里时也不标别的
    const outside = build({
      active: "route",
      routeId: "backup",
      failoverOn: true,
      queue: ["backup"],
    });
    expect(item(outside.sections, "relay")).toMatchObject({
      section: "rest",
      chips: [],
    });
  });

  it("numbers the queue by its full order and marks the recorded route, not the first visible card", () => {
    // 搜索把 P1 过滤掉了：剩下的卡序号、上下移仍按完整队列算
    const { sections, input } = build({
      active: "route",
      routeId: "backup",
      failoverOn: true,
      queue: ["relay", "backup", "converted"],
      providers: [backup, converted],
    });

    expect(item(sections, "backup").chips).toEqual([
      { key: "priority", label: "P2", tone: "outline" },
    ]);
    expect(item(sections, "converted").chips[0].label).toBe("P3");
    item(sections, "backup").move?.onUp?.();
    expect(input.actions.queueMove).toHaveBeenCalledWith(backup, -1);
    expect(item(sections, "converted").move?.onDown).toBeUndefined();

    // 转移成功后后端记下的路由是 backup：「路由中」在它身上，不在队首
    expect(item(sections, "backup")).toMatchObject({
      tone: "route",
      status: { label: "providerCard.status.routing" },
    });
    expect(item(sections, "converted").status).toBeUndefined();

    // 记下的路由已不在队列里：退回队首
    const fallback = build({
      active: "route",
      routeId: "official",
      failoverOn: true,
      queue: ["relay", "backup"],
    });
    expect(item(fallback.sections, "relay").tone).toBe("route");
    expect(item(fallback.sections, "backup").tone).toBeUndefined();
  });

  it("keeps Codex official cards out of the failover queue with a reason", () => {
    // 后端 require_failover_provider 会拒绝；卡上要先说清楚，而不是点了没反应
    const card = provider("legacy-official", {
      id: "codex-official",
      category: "official",
      settingsConfig: { auth: {}, config: "" },
    } as Partial<Provider>);
    const { sections, input } = build({
      app: "codex",
      active: "route",
      routeId: "relay",
      failoverOn: true,
      queue: ["relay"],
      providers: [relay, backup, card],
    });

    expect(item(sections, "codex-official").dim).toBe(true);
    expect(button(sections, "codex-official", "blocked").disabledReason).toBe(
      "providerCard.reason.noFailover",
    );
    button(sections, "backup", "queueAdd").onClick();
    expect(input.actions.queueAdd).toHaveBeenCalledWith(backup);
  });
});

describe("buildDesktopSections", () => {
  it("colors the current card by whether it uses model mapping", () => {
    const mapped = provider("mapped", { meta: { claudeDesktopMode: "proxy" } });
    const onSwitch = vi.fn();
    const sections = buildDesktopSections({
      t,
      providers: [mapped, relay],
      currentId: "mapped",
      onSwitch,
    });

    expect(item(sections, "mapped")).toMatchObject({
      tone: "route",
      status: { label: "providerCard.status.inUse", dot: "route" },
      deleteDisabledReason: "providerCard.reason.inUseCannotDelete",
    });
    expect(item(sections, "relay").chips.map((c) => c.key)).toEqual(["direct"]);
    expect(item(sections, "relay").deleteDisabledReason).toBeUndefined();
    button(sections, "relay", "switch").onClick();
    expect(onSwitch).toHaveBeenCalledWith(relay);
  });
});
