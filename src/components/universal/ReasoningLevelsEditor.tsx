import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronsUpDown } from "lucide-react";
import { HoverTip } from "@/components/ui/hover-tip";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

/**
 * effort 档位全集（升序深度）。后端会丢弃未知值，UI 只给预置档位。
 */
export const REASONING_LEVELS = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
] as const;

/**
 * 触发器标签：相邻 ≥3 个的连续预置档位压成「首 → 末」（跳过 minimal 时是
 * "none, low → max"）。范围不会跨过未选档位，所以不会暗示没选的档。
 * 非预置顺序保持原样列表。
 */
export function formatReasoningLevelsLabel(levels: string[]): string {
  const canonical = REASONING_LEVELS as readonly string[];
  const positions = levels.map((level) => canonical.indexOf(level));
  const ordered = positions.every(
    (position, index) =>
      position >= 0 && (index === 0 || position > positions[index - 1]),
  );
  if (!ordered) return levels.join(", ");

  const parts: string[] = [];
  let runStart = 0;
  for (let index = 1; index <= levels.length; index++) {
    if (
      index < levels.length &&
      positions[index] === positions[index - 1] + 1
    ) {
      continue;
    }
    const run = levels.slice(runStart, index);
    if (run.length >= 3) {
      parts.push(`${run[0]} → ${run[run.length - 1]}`);
    } else {
      parts.push(...run);
    }
    runStart = index;
  }
  return parts.join(", ");
}

// Radix Select 禁止空 value 项：「回到自动」用一个哨兵值映射成 undefined。
const AUTO_DEFAULT_REASONING_LEVEL = "__auto__";

/**
 * 逐模型的 effort 档位编辑器：多选下拉（可搜索）+ 默认档位选择。
 *
 * 勾选顺序无关——存的是预置升序；列表为空表示「未设置」（目录不写该字段）。
 */
export function ReasoningLevelsEditor({
  levels,
  defaultLevel,
  onLevelsChange,
  onDefaultLevelChange,
}: {
  levels?: string[];
  defaultLevel?: string;
  onLevelsChange: (levels: string[] | undefined) => void;
  onDefaultLevelChange: (level: string | undefined) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  // 拉取的档位来自 oh-my-pi 模型目录的声明顺序（merge_efforts 保持首现顺序），可能不是预置
  // 升序。读侧同样按预置序规范化：触发器标签、默认档下拉、勾选集合都以它为准，和写入侧
  // （toggleLevel）的排序一致。非预置值会被过滤——列表本来就是从预置里勾的，存进库的
  // 也一定是预置；万一混进未知值，过滤只是不显示它，不丢数据。
  const selected = (REASONING_LEVELS as readonly string[]).filter((level) =>
    (levels ?? []).includes(level),
  );

  const toggleLevel = (level: string) => {
    const picked = selected.includes(level)
      ? selected.filter((item) => item !== level)
      : [...selected, level];
    // 存预置升序（不是点击顺序）：Codex 选择器和生成的目录都按数组顺序。
    const next = (REASONING_LEVELS as readonly string[]).filter((item) =>
      picked.includes(item),
    );
    onLevelsChange(next.length > 0 ? next : undefined);
    if (defaultLevel && !next.includes(defaultLevel)) {
      onDefaultLevelChange(undefined);
    }
  };

  const triggerLabel =
    selected.length > 0
      ? formatReasoningLevelsLabel(selected)
      : t("reasoningLevels.notSet", { defaultValue: "未设置" });

  // 标签可能是范围或被截断，悬停把选中的档位与默认档全部展开。
  const triggerTip =
    selected.length > 0 ? (
      <>
        <div>{selected.join(", ")}</div>
        {defaultLevel && (
          <div>
            {t("reasoningLevels.defaultTip", {
              level: defaultLevel,
              defaultValue: "默认档位：{{level}}",
            })}
          </div>
        )}
      </>
    ) : undefined;

  return (
    <Popover modal open={open} onOpenChange={setOpen}>
      <HoverTip content={triggerTip}>
        <PopoverTrigger asChild>
          <button
            type="button"
            role="combobox"
            aria-expanded={open}
            className="flex h-9 w-full items-center justify-between gap-1 rounded-md border border-border bg-surface px-3 py-1 text-sm shadow-sm focus:outline-none focus-visible:outline-none focus:border-border focus-visible:border-border focus:ring-0 focus-visible:ring-0 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <span
              className={cn("truncate", selected.length === 0 && "text-fg-2")}
            >
              {triggerLabel}
            </span>
            <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 opacity-50" />
          </button>
        </PopoverTrigger>
      </HoverTip>
      <PopoverContent
        side="bottom"
        align="start"
        sideOffset={6}
        avoidCollisions
        collisionPadding={8}
        className="z-[1000] w-[var(--radix-popover-trigger-width)] p-0 border-border"
      >
        <Command>
          <CommandInput
            placeholder={t("reasoningLevels.search", {
              defaultValue: "搜索思考等级...",
            })}
          />
          <CommandList>
            <CommandEmpty>
              {t("reasoningLevels.empty", { defaultValue: "没有匹配的档位" })}
            </CommandEmpty>
            <CommandGroup>
              {REASONING_LEVELS.map((level) => (
                <CommandItem
                  key={level}
                  value={level}
                  onSelect={() => toggleLevel(level)}
                >
                  <Check
                    className={cn(
                      "mr-2 h-4 w-4",
                      selected.includes(level) ? "opacity-100" : "opacity-0",
                    )}
                  />
                  <span className="flex-1">{level}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
        {selected.length > 0 && (
          <div className="border-t border-border p-2">
            <span className="text-xs text-fg-2">
              {t("reasoningLevels.defaultLabel", { defaultValue: "默认档位" })}
            </span>
            <Select
              value={defaultLevel ?? AUTO_DEFAULT_REASONING_LEVEL}
              onValueChange={(value) =>
                onDefaultLevelChange(
                  value === AUTO_DEFAULT_REASONING_LEVEL ? undefined : value,
                )
              }
            >
              <SelectTrigger className="mt-1 h-8 w-full">
                <SelectValue
                  placeholder={t("reasoningLevels.auto", {
                    defaultValue: "自动",
                  })}
                />
              </SelectTrigger>
              {/* 必须盖过外层 z-[1000] 的 popover：SelectContent 默认 z-[100]
                  会在面板上翻时被遮住。 */}
              <SelectContent className="z-[1100]">
                <SelectItem value={AUTO_DEFAULT_REASONING_LEVEL}>
                  {t("reasoningLevels.auto", { defaultValue: "自动" })}
                </SelectItem>
                {selected.map((level) => (
                  <SelectItem key={level} value={level}>
                    {level}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
