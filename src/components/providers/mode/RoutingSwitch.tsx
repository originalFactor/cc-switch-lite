import { useTranslation } from "react-i18next";
import type { Provider } from "@/types";
import type { AppMode } from "@/types/proxy";
import type { ProxyAppId } from "@/config/appConfig";
import { Switch } from "@/components/ui/switch";
import { HoverTip } from "@/components/ui/hover-tip";
import { APP_DISPLAY_NAME } from "@/components/shell/AppGlyph";
import { cn } from "@/lib/utils";

interface RoutingSwitchProps {
  app: ProxyAppId;
  /** 现在生效的模式：`route` 时开关是开的。 */
  active: AppMode;
  /** 直连那家：打开开关时路由到它。 */
  directProvider: Provider | undefined;
  /** 现在的路由目标（没选过时为空）。 */
  routeProvider: Provider | undefined;
  /** 不能打开的原因（例如这家官方订阅不能走路由、没有当前供应商）：显示在悬停说明里。 */
  disabledReason?: string;
  pending?: boolean;
  onChange: (next: boolean) => void;
}

/**
 * 应用页页头的路由开关：开着 = 请求经本机路由；关掉 = 直接连当前供应商。
 *
 * 打开时用「现在直连的那家」做路由目标（用户没改过直连指针，关掉后同一家回来），所以
 * 这家不能走路由时开关是灰的，原因写在悬停说明里；换路由目标仍在路由设置里。
 */
export function RoutingSwitch({
  app,
  active,
  directProvider,
  routeProvider,
  disabledReason,
  pending = false,
  onChange,
}: RoutingSwitchProps) {
  const { t } = useTranslation();
  const appName = APP_DISPLAY_NAME[app];
  const checked = active === "route";
  const target =
    active === "route" ? (routeProvider ?? directProvider) : undefined;
  const tip = disabledReason
    ? `${t("mode.routingSwitchTip", { app: appName })} — ${disabledReason}`
    : t("mode.routingSwitchTip", { app: appName });

  return (
    <HoverTip content={tip}>
      <label
        className={cn(
          "flex shrink-0 items-center gap-2",
          disabledReason || pending ? "cursor-default" : "cursor-pointer",
        )}
      >
        <span className="whitespace-nowrap text-body font-medium text-fg-2">
          {t("mode.names.route")}
          {target ? ` · ${target.name}` : ""}
        </span>
        <Switch
          checked={checked}
          disabled={pending || Boolean(disabledReason)}
          onCheckedChange={(next) => onChange(next)}
          aria-label={t("mode.routingSwitchAria", { app: appName })}
        />
      </label>
    </HoverTip>
  );
}
