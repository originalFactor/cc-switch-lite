import { describe, expect, it } from "vitest";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";

type TranslationTree = Record<string, unknown>;

function flattenStrings(
  value: unknown,
  path: string[] = [],
  result = new Map<string, string>(),
): Map<string, string> {
  if (typeof value === "string") {
    result.set(path.join("."), value);
  } else if (typeof value === "object" && value !== null) {
    for (const [key, child] of Object.entries(value)) {
      flattenStrings(child, [...path, key], result);
    }
  }
  return result;
}

function interpolationVariables(value: string): string[] {
  return Array.from(
    value.matchAll(/\{\{\s*([^}]+?)\s*\}\}/g),
    ([, name]) => name,
  ).sort();
}

const reference = flattenStrings(en);
const locales = [["zh", zh]] as const;

/**
 * 这几条文案各语言刻意用不同的占位符表达同一件事：zh 用 `{{month}}/{{day}}` 而非
 * `{{date}}`，中文排版用 `{{labelSp}}` 而非 `{{label}}`。
 * 逐个点名豁免，而不是放宽整张表的比较——否则新增的插值漂移会被一起放过。
 */
const LANGUAGE_SPECIFIC_PLACEHOLDERS = new Set([
  "prompts.updatedOn",
  "quota.tierLeft",
  "quota.tierUsedUp",
  "quota.tierUsed",
]);

/**
 * 运行时缺键会安静地回退英文，构建也不会失败，所以键集合与插值变量的一致性只能由
 * 测试守住：少一个键就是一句英文，少一个 `{{x}}` 就是一句残缺的话。
 */
describe("locale coverage", () => {
  it.each(locales)("covers every English key in %s", (_name, tree) => {
    const translations = flattenStrings(tree as TranslationTree);
    const missing = [...reference.keys()].filter(
      (key) => !translations.has(key),
    );

    expect(missing).toEqual([]);
  });

  it.each(locales)("has no unexpected extra key in %s", (_name, tree) => {
    const translations = flattenStrings(tree as TranslationTree);
    const extra = [...translations.keys()].filter(
      (key) => !reference.has(key),
    );

    expect(extra).toEqual([]);
  });

  it.each(locales)(
    "preserves every interpolation variable in %s",
    (_name, tree) => {
      const translations = flattenStrings(tree as TranslationTree);
      const mismatched = [...reference].flatMap(([key, expected]) => {
        if (LANGUAGE_SPECIFIC_PLACEHOLDERS.has(key)) return [];
        const actual = translations.get(key);
        return actual !== undefined &&
          interpolationVariables(actual).join("\0") !==
            interpolationVariables(expected).join("\0")
          ? [key]
          : [];
      });

      expect(mismatched).toEqual([]);
    },
  );
});
