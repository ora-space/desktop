import type { PlatformLocale } from "../types";

export interface PlatformMessages {
  cancel: string;
  choose: string;
  chooseCurrentDirectory: string;
  chooseDirectoryTitle: string;
  chooseFileTitle: string;
  emptyDirectory: string;
  go: string;
  loading: string;
  pathLabel: string;
  readError: string;
  retry: string;
  up: string;
}

const messages: Record<PlatformLocale, PlatformMessages> = {
  "zh-CN": {
    cancel: "取消",
    choose: "选择",
    chooseCurrentDirectory: "选择当前文件夹",
    chooseDirectoryTitle: "选择文件夹",
    chooseFileTitle: "选择文件",
    emptyDirectory: "此文件夹为空",
    go: "前往",
    loading: "正在读取文件夹…",
    pathLabel: "绝对路径",
    readError: "无法读取该文件夹，请检查路径或权限。",
    retry: "重试",
    up: "上一级",
  },
  "en-US": {
    cancel: "Cancel",
    choose: "Select",
    chooseCurrentDirectory: "Select current folder",
    chooseDirectoryTitle: "Select folder",
    chooseFileTitle: "Select file",
    emptyDirectory: "This folder is empty",
    go: "Go",
    loading: "Loading folder…",
    pathLabel: "Absolute path",
    readError: "Unable to read this folder. Check the path or permissions.",
    retry: "Retry",
    up: "Up one level",
  },
};

/** Returns the platform-owned strings for the host application's current locale. */
export function platformMessages(locale: PlatformLocale): PlatformMessages {
  return messages[locale];
}
