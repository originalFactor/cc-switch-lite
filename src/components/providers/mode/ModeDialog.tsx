import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";
import type { Provider } from "@/types";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import { logFrontendInfo } from "@/lib/frontendLogger";

/** 路由模式改写的客户端文件（确认框里写明，用默认位置）。 */
const CLIENT_FILE: Partial<Record<AppId, string>> = {
  claude: "~/.claude/settings.json",
  codex: "~/.codex/config.toml",
  gemini: "~/.gemini/.env",
  grokbuild: "~/.grok/config.toml",
};

export type ModeDialogState = {
  kind: "needsRoute";
  providerId: string;
  reason: string;
};

interface ModeDialogProps {
  app: AppId;
  state: ModeDialogState | null;
  providers: Provider[];
  onClose: () => void;
  /** 开始路由并使用这家。 */
  onRoute: (providerId: string) => void;
  /** 对话框 F：仍然直连切换 */
  onSwitchDirect: (providerId: string) => void;
}

/**
 * 「这家需要路由」的确认框：不经过页面头的开关，直接为这家开路由，或者仍然直连切换。
 * 开关是打开路由的唯一入口；这里只是把「点了需要路由的卡片」这一步讲清楚。
 */
export function ModeDialog({
  app,
  state,
  providers,
  onClose,
  onRoute,
  onSwitchDirect,
}: ModeDialogProps) {
  const { t } = useTranslation();

  useEffect(() => {
    if (!state) return;
    logFrontendInfo(
      `[MODE] ${app} 「${state.providerId} 需要路由」确认框：弹出`,
    );
  }, [app, state]);

  const provider = state
    ? providers.find((p) => p.id === state.providerId)
    : undefined;
  const name = provider?.name ?? state?.providerId ?? "";
  const appName = APP_DISPLAY_NAME[app];

  const cancel = () => {
    if (state) {
      logFrontendInfo(
        `[MODE] ${app} 「${state.providerId} 需要路由」确认框：取消`,
      );
    }
    onClose();
  };

  return (
    <Dialog open={state !== null} onOpenChange={(open) => !open && cancel()}>
      {state && (
        <DialogContent
          zIndex="alert"
          className="max-w-[480px] gap-0 rounded-dialog border-border bg-surface p-6 shadow-v7-lg sm:rounded-dialog"
        >
          <div className="space-y-4">
            <div className="space-y-1">
              <DialogTitle className="text-title">
                {t("mode.dialog.needsRouteTitle", { provider: name })}
              </DialogTitle>
              <DialogDescription className="text-body text-fg-2">
                {t("mode.dialog.needsRouteLead", {
                  provider: name,
                  reason: state.reason,
                  app: appName,
                })}
              </DialogDescription>
            </div>
            <p className="rounded-panel bg-subtle px-4 py-3 text-caption text-fg-2">
              {t("mode.dialog.routeNoteFile", {
                file: CLIENT_FILE[app] ?? "",
              })}
            </p>
            <div className="flex flex-wrap justify-end gap-2 pt-1">
              <Button
                variant="neutral"
                size="regular"
                autoFocus
                onClick={cancel}
              >
                {t("common.cancel")}
              </Button>
              <Button
                variant="neutral"
                size="regular"
                onClick={() => {
                  logFrontendInfo(
                    `[MODE] ${app} 「${state.providerId} 需要路由」确认框：仍然直连切换`,
                  );
                  onClose();
                  onSwitchDirect(state.providerId);
                }}
              >
                {t("mode.dialog.switchAnyway")}
              </Button>
              <Button
                variant="route"
                size="regular"
                onClick={() => {
                  logFrontendInfo(
                    `[MODE] ${app} 「${state.providerId} 需要路由」确认框：开始路由并使用`,
                  );
                  onRoute(state.providerId);
                  onClose();
                }}
              >
                {t("mode.dialog.routeAndUse")}
              </Button>
            </div>
          </div>
        </DialogContent>
      )}
    </Dialog>
  );
}
