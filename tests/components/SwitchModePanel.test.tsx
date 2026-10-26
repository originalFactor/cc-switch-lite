import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { toast } from "sonner";
import type { Provider } from "@/types";
import type { ProxyAppId } from "@/config/appConfig";
import type { AppMode } from "@/types/proxy";
import { SwitchModePanel } from "@/components/providers/mode/SwitchModePanel";
import type { CardButton, CardPresentation } from "@/components/providers/presentation";
import { server } from "../msw/server";
import { createTestQueryClient } from "../utils/testQueryClient";

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    info: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
  },
}));

// 只关心模式层：卡片画成按钮，按 presentation 的 key 找
vi.mock("@/components/providers/ProviderCard", () => ({
  ProviderCard: ({
    provider,
    presentation,
  }: {
    provider: Provider;
    presentation: CardPresentation;
  }) => (
    <div data-testid={`card-${provider.id}`}>
      {presentation.buttons.map((button: CardButton) => (
        <button
          key={button.key}
          data-testid={`${button.key}-${provider.id}`}
          disabled={Boolean(button.disabledReason)}
          onClick={button.onClick}
        >
          {button.label}
        </button>
      ))}
    </div>
  ),
}));

vi.mock("@/hooks/useStreamCheck", () => ({
  useStreamCheck: () => ({ checkProvider: vi.fn(), isChecking: () => false }),
}));

const TAURI_ENDPOINT = "http://tauri.local";

const provider = (id: string): Provider => ({
  id,
  name: id,
  settingsConfig: {},
});

function mockMode(
  mode: AppMode,
  routeProviderId: string | null,
  directProviderId: string | null = routeProviderId,
) {
  server.use(
    http.post(`${TAURI_ENDPOINT}/get_app_mode`, () =>
      HttpResponse.json({
        mode,
        attached: mode !== "direct",
        routeProviderId,
        directProviderId,
      }),
    ),
    http.post(`${TAURI_ENDPOINT}/get_proxy_status`, () =>
      HttpResponse.json({ running: true }),
    ),
  );
}

function mockStaleClients(
  staleClients: { daemon?: boolean; others?: boolean; auth?: boolean },
  revision = "r1",
) {
  server.use(
    http.post(`${TAURI_ENDPOINT}/get_codex_stale_clients`, () =>
      HttpResponse.json({
        staleClients: { daemon: false, others: false, ...staleClients },
        staleRevision: revision,
      }),
    ),
  );
}

function renderPanel(
  app: ProxyAppId,
  providers: Record<string, Provider>,
  onSwitch = vi.fn(),
) {
  const queryClient = createTestQueryClient();
  const view = render(
    <QueryClientProvider client={queryClient}>
      <SwitchModePanel
        app={app}
        providers={providers}
        currentProviderId={Object.keys(providers)[0] ?? ""}
        isLoading={false}
        onSwitch={onSwitch}
        onDelete={vi.fn()}
        onOpenWebsite={vi.fn()}
      />
    </QueryClientProvider>,
  );
  return { ...view, queryClient };
}

beforeEach(() => {
  vi.mocked(toast.info).mockClear();
  vi.mocked(toast.success).mockClear();
  // jsdom 没有 scrollIntoView，确认框里的下拉打开时会调
  Element.prototype.scrollIntoView = vi.fn();
});

describe("SwitchModePanel — single list", () => {
  it("shows one list: switch on the direct card, route here in routing mode", async () => {
    mockMode("direct", "backup", "relay");
    const onSwitch = vi.fn();
    const view = renderPanel(
      "claude",
      { relay: provider("relay"), backup: provider("backup") },
      onSwitch,
    );

    expect(await screen.findByTestId("card-relay")).toBeInTheDocument();
    expect(screen.queryByRole("tab")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("switch-backup"));
    expect(onSwitch).toHaveBeenCalledWith(
      expect.objectContaining({ id: "backup" }),
    );
    view.unmount();

    mockMode("route", "backup", "relay");
    renderPanel("claude", { relay: provider("relay"), backup: provider("backup") });
    fireEvent.click(await screen.findByTestId("routeHere-relay"));
  });

  it("shows queue sections while failover is on", async () => {
    mockMode("route", "backup", "relay");
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_failover_queue`, () =>
        HttpResponse.json([
          { appType: "claude", providerId: "backup", priority: 1 },
        ]),
      ),
      http.post(`${TAURI_ENDPOINT}/get_auto_failover_enabled`, () =>
        HttpResponse.json(true),
      ),
    );

    renderPanel("claude", {
      relay: provider("relay"),
      backup: provider("backup"),
    });
    expect(
      await screen.findByRole("heading", { name: "providerCard.section.queue" }),
    ).toBeInTheDocument();
  });

  it("offers the needs-route dialog from a card that needs routing", async () => {
    mockMode("direct", null, "relay");
    const converted: Provider = {
      ...provider("converted"),
      meta: { apiFormat: "openai_chat" },
    };
    renderPanel("claude", { relay: provider("relay"), converted });

    fireEvent.click(await screen.findByTestId("switch-converted"));
    expect(
      await screen.findByText("mode.dialog.needsRouteTitle"),
    ).toBeInTheDocument();
  });
});

describe("SwitchModePanel — Codex stale clients", () => {
  it("warns about Codex clients on an old model list in both modes", async () => {
    mockMode("direct", "route");
    mockStaleClients({ daemon: true });
    const view = renderPanel("codex", { route: provider("route") });
    expect(
      await screen.findByText("proxy.codexStale.title"),
    ).toBeInTheDocument();
    view.unmount();

    mockMode("route", "route");
    renderPanel("codex", { route: provider("route") });
    expect(
      await screen.findByText("proxy.codexStale.title"),
    ).toBeInTheDocument();
  });

  it("shows a dismissed notice again once the model list changes again", async () => {
    let revision = "r1";
    mockMode("direct", "route");
    mockStaleClients({ others: true }, revision);
    const { queryClient } = renderPanel("codex", { route: provider("route") });
    await screen.findByText("proxy.codexStale.title");
    fireEvent.click(screen.getByRole("button", { name: "common.close" }));
    expect(
      screen.queryByText("proxy.codexStale.title"),
    ).not.toBeInTheDocument();

    // 同一份：再拉一次还是关着。
    await queryClient.invalidateQueries({
      queryKey: ["providers", "codex", "codexStaleClients"],
    });
    expect(
      screen.queryByText("proxy.codexStale.title"),
    ).not.toBeInTheDocument();

    // 目录又改了一次：同样的客户端，但已经是新的一份。
    revision = "r2";
    mockStaleClients({ others: true }, revision);
    await queryClient.invalidateQueries({
      queryKey: ["providers", "codex", "codexStaleClients"],
    });
    expect(
      await screen.findByText("proxy.codexStale.title"),
    ).toBeInTheDocument();
  });

  it.each(["direct", "route"] as const)(
    "shows a cached account notice in %s mode",
    async (mode) => {
      mockMode(mode, "route");
      mockStaleClients({ daemon: true, auth: true });
      renderPanel("codex", { route: provider("route") });
      expect(
        await screen.findByText("proxy.codexStale.authTitle"),
      ).toBeInTheDocument();
      expect(
        screen.queryByText("proxy.codexStale.title"),
      ).not.toBeInTheDocument();
    },
  );

  it("does not read stale clients for apps other than Codex", async () => {
    mockMode("route", "a");
    let reads = 0;
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_codex_stale_clients`, () => {
        reads += 1;
        return HttpResponse.json(null);
      }),
    );

    renderPanel("gemini", { a: provider("a") });
    await screen.findByTestId("card-a");
    expect(reads).toBe(0);
  });
});

describe("SwitchModePanel — routing states", () => {
  it("keeps a single list while routed", async () => {
    mockMode("route", "route");
    renderPanel("gemini", { route: provider("route") });
    expect(
      await screen.findByTestId("card-route"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("tab")).not.toBeInTheDocument();
  });

  it("shows the service-down notice with a back-to-direct action", async () => {
    mockMode("route", "route");
    server.use(
      http.post(`${TAURI_ENDPOINT}/get_proxy_status`, () =>
        HttpResponse.json({ running: false }),
      ),
    );
    const calls: unknown[] = [];
    server.use(
      http.post(
        `${TAURI_ENDPOINT}/set_proxy_takeover_for_app`,
        async ({ request }) => {
          calls.push(await request.json());
          return HttpResponse.json(null);
        },
      ),
    );
    renderPanel("claude", { route: provider("route") });
    expect(
      await screen.findByText("mode.notice.serviceDown"),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "mode.notice.backToDirect" }),
    );
    await waitFor(() =>
      expect(calls).toEqual([{ appType: "claude", enabled: false, route: null }]),
    );
  });
});

describe("SwitchModePanel — tray needs-route request", () => {
  it("opens the same needs-routing dialog once the provider is there", async () => {
    mockMode("direct", null, "kimi");
    const onHandled = vi.fn();
    const copilot: Provider = {
      ...provider("copilot"),
      meta: { providerType: "github_copilot" },
    };
    const queryClient = createTestQueryClient();
    render(
      <QueryClientProvider client={queryClient}>
        <SwitchModePanel
          app="claude"
          providers={{ kimi: provider("kimi"), copilot }}
          currentProviderId="kimi"
          isLoading={false}
          onSwitch={vi.fn()}
          onDelete={vi.fn()}
          onOpenWebsite={vi.fn()}
          needsRouteRequest={{ providerId: "copilot", nonce: 1 }}
          onNeedsRouteHandled={onHandled}
        />
      </QueryClientProvider>,
    );

    expect(
      await screen.findByText("mode.dialog.needsRouteTitle"),
    ).toBeInTheDocument();
    expect(onHandled).toHaveBeenCalledTimes(1);
  });
});
