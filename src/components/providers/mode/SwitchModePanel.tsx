import { useEffect, useMemo, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "@/lib/toast";
import type { Provider } from "@/types";
import type { StartupAttachFailure } from "@/types/proxy";
import type { ProxyAppId } from "@/config/appConfig";
import { proxyApi } from "@/lib/api/proxy";
import { providersApi } from "@/lib/api/providers";
import {
  proxyKeys,
  useAppMode,
  useCodexStaleClients,
  useProxyStatusQuery,
} from "@/lib/query/proxy";
import {
  useAddToFailoverQueue,
  useAutoFailoverEnabled,
  useFailoverQueue,
  useRemoveFromFailoverQueue,
} from "@/lib/query/failover";
import { useModeActions } from "@/hooks/useModeActions";
import { getRoutingReason } from "@/utils/routingReason";
import { extractErrorMessage } from "@/utils/errorUtils";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/ui/notice";
import { ProviderList } from "@/components/providers/ProviderList";
import { CodexStaleClientsNotice } from "@/components/providers/CodexStaleClientsNotice";
import { APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import { ModeDialog, type ModeDialogState } from "./ModeDialog";

type ListCallbacks = Pick<
  React.ComponentProps<typeof ProviderList>,
  | "onDelete"
  | "onConfigureUsage"
  | "onOpenWebsite"
  | "onOpenTerminal"
  | "onEditUniversal"
  | "onCreate"
  | "searchOpen"
  | "onSearchOpenChange"
>;

interface SwitchModePanelProps extends ListCallbacks {
  app: ProxyAppId;
  providers: Record<string, Provider>;
  currentProviderId: string;
  isLoading: boolean;
  scrollRef?: RefObject<HTMLDivElement>;
  onSwitch: (
    provider: Provider,
    options?: { acknowledgedRouting?: boolean },
  ) => unknown;
  startupFailure?: StartupAttachFailure;
  onDismissStartupFailure?: () => void;
  /** 托盘里点了直连下需要路由的那家：到这页后弹同一个「需要路由」对话框 */
  needsRouteRequest?: { providerId: string; nonce: number };
  onNeedsRouteHandled?: () => void;
}

/**
 * 切换式应用的供应商页：一个列表，加通知槽。直连 / 路由由页头的开关切换；列表里的卡片
 * 在直连时切换直连供应商，在路由时换路由目标。
 */
export function SwitchModePanel({
  app,
  providers,
  currentProviderId,
  isLoading,
  scrollRef,
  onSwitch,
  startupFailure,
  onDismissStartupFailure,
  needsRouteRequest,
  onNeedsRouteHandled,
  ...listCallbacks
}: SwitchModePanelProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const appName = APP_DISPLAY_NAME[app];

  const { data: modeView } = useAppMode(app);
  const active = modeView?.mode ?? "direct";
  const directId = modeView?.directProviderId ?? null;
  const routeId = modeView?.routeProviderId ?? null;

  const { data: proxyStatus } = useProxyStatusQuery();
  const serviceRunning = proxyStatus?.running ?? false;
  const { data: failoverOn = false } = useAutoFailoverEnabled(app);
  const { data: queue } = useFailoverQueue(app);
  const addToQueue = useAddToFailoverQueue();
  const removeFromQueue = useRemoveFromFailoverQueue();
  const { data: staleClientsView } = useCodexStaleClients(app, app === "codex");
  const modeActions = useModeActions(app);

  const [dialog, setDialog] = useState<ModeDialogState | null>(null);
  // 关掉的是哪一份提示（后端的 staleRevision）：目录或登录又变了就是新的一份，重新显示。
  const [staleDismissed, setStaleDismissed] = useState<string | null>(null);
  useEffect(() => {
    setStaleDismissed(null);
  }, [app, active, directId, routeId, staleClientsView?.staleClients.auth]);
  const staleRevision = staleClientsView?.staleRevision ?? "";

  // 供应商还没加载完时先等着，到了再弹
  useEffect(() => {
    if (!needsRouteRequest) return;
    const provider = providers[needsRouteRequest.providerId];
    if (!provider) return;
    setDialog({
      kind: "needsRoute",
      providerId: provider.id,
      reason: getRoutingReason(app, provider, t) ?? "",
    });
    onNeedsRouteHandled?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [needsRouteRequest?.nonce, providers]);

  const providerList = useMemo(() => Object.values(providers), [providers]);
  const nameOf = (id: string | null) =>
    (id && providers[id]?.name) || t("mode.noProvider");

  const queueIds = useMemo(
    () => (queue ?? []).map((item) => item.providerId),
    [queue],
  );

  const previous = { mode: active, routeProviderId: routeId };

  const queueMove = async (provider: Provider, delta: -1 | 1) => {
    const items = queue ?? [];
    const index = items.findIndex((item) => item.providerId === provider.id);
    const target = index + delta;
    if (index < 0 || target < 0 || target >= items.length) return;
    const order = items.map((item) => item.providerId);
    order.splice(index, 1);
    order.splice(target, 0, provider.id);
    // 队列顺序就是供应商的排序：按原来占着的位置重新分配
    const slots = items
      .map(
        (item, i) =>
          item.sortIndex ?? providers[item.providerId]?.sortIndex ?? i,
      )
      .sort((a, b) => a - b);
    try {
      await providersApi.updateSortOrder(
        order.map((id, i) => ({ id, sortIndex: slots[i] })),
        app,
      );
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["failoverQueue", app] }),
        queryClient.invalidateQueries({ queryKey: ["providers", app] }),
      ]);
      toast.success(
        t("mode.queueMoved", { name: provider.name, position: target + 1 }),
      );
    } catch (error) {
      toast.error(extractErrorMessage(error) || t("provider.sortUpdateFailed"));
    }
  };

  const switchMode = {
    active,
    directId,
    routeId,
    failoverOn,
    queue: queueIds,
    serviceRunning,
    routingReason: (provider: Provider) =>
      getRoutingReason(app, provider, t) ?? "",
    actions: {
      switchDirect: (provider: Provider) => void onSwitch(provider),
      needsRouteDialog: (provider: Provider) =>
        setDialog({
          kind: "needsRoute",
          providerId: provider.id,
          reason: getRoutingReason(app, provider, t) ?? "",
        }),
      routeTo: (provider: Provider) =>
        void onSwitch(provider, { acknowledgedRouting: true }),
      queueAdd: (provider: Provider) =>
        addToQueue.mutate({ appType: app, providerId: provider.id }),
      queueRemove: (provider: Provider) =>
        removeFromQueue.mutate({ appType: app, providerId: provider.id }),
      queueMove: (provider: Provider, delta: -1 | 1) =>
        void queueMove(provider, delta),
    },
  };

  const startService = async () => {
    try {
      await proxyApi.startProxyServer();
      await queryClient.invalidateQueries({ queryKey: proxyKeys.status });
    } catch (error) {
      toast.error(
        t("proxy.server.startFailed", {
          detail: extractErrorMessage(error) || t("common.unknown"),
        }),
      );
    }
  };

  const notices: React.ReactNode[] = [];
  if (active !== "direct" && proxyStatus !== undefined && !serviceRunning) {
    notices.push(
      <Notice
        key="broken"
        tone="warning"
        title={t("mode.notice.serviceDown", {
          app: appName,
          mode: t(`mode.names.${active}`),
        })}
        actions={
          <>
            <Button
              variant="neutral"
              size="compact"
              onClick={() => void startService()}
            >
              {t("mode.notice.startService")}
            </Button>
            <Button
              variant="neutral"
              size="compact"
              onClick={() =>
                void modeActions.exitToDirect({
                  previous,
                  directProviderId: directId,
                  providerName: nameOf(directId),
                })
              }
            >
              {t("mode.notice.backToDirect")}
            </Button>
          </>
        }
      />,
    );
  }
  if (startupFailure) {
    notices.push(
      <Notice
        key="startup"
        tone="warning"
        title={t("mode.notice.attachFailed", {
          app: appName,
          mode: t("mode.names.route"),
        })}
        onDismiss={onDismissStartupFailure}
        dismissLabel={t("common.close")}
        actions={
          <Button
            variant="neutral"
            size="compact"
            onClick={() =>
              void modeActions
                .enter(routeId, nameOf(routeId))
                .then(() => onDismissStartupFailure?.())
                .catch((error) =>
                  toast.error(
                    t("mode.toast.failed", {
                      detail: extractErrorMessage(error) || t("common.unknown"),
                    }),
                  ),
                )
            }
          >
            {t("common.retry")}
          </Button>
        }
      >
        {startupFailure.error}
      </Notice>,
    );
  }
  if (
    app === "codex" &&
    staleClientsView?.staleClients &&
    (staleClientsView.staleClients.daemon ||
      staleClientsView.staleClients.others) &&
    staleDismissed !== staleRevision
  ) {
    notices.push(
      <CodexStaleClientsNotice
        key="stale"
        staleClients={staleClientsView.staleClients}
        onDismiss={() => setStaleDismissed(staleRevision)}
      />,
    );
  }

  return (
    <>
      <div
        ref={scrollRef}
        id="main-content"
        className="min-h-0 flex-1 overflow-y-auto scroll-stable overflow-x-hidden px-6 pb-12 pt-4"
      >
        {notices.length > 0 && (
          <div role="status" className="mb-3 flex flex-col gap-2">
            {notices}
          </div>
        )}
        <ProviderList
          {...listCallbacks}
          providers={providers}
          currentProviderId={currentProviderId}
          appId={app}
          isLoading={isLoading}
          onSwitch={(provider) => void onSwitch(provider)}
          switchMode={switchMode}
        />
      </div>

      <ModeDialog
        app={app}
        state={dialog}
        providers={providerList}
        onClose={() => setDialog(null)}
        onRoute={(providerId) => {
          void modeActions
            .enter(providerId, providers[providerId]?.name)
            .catch((error) =>
              toast.error(
                t("mode.toast.failed", {
                  detail: extractErrorMessage(error) || t("common.unknown"),
                }),
              ),
            );
        }}
        onSwitchDirect={(providerId) => {
          const provider = providers[providerId];
          if (provider) void onSwitch(provider, { acknowledgedRouting: true });
        }}
      />
    </>
  );
}
