// 网页打不开访客电脑上的文件和文件夹。选文件、打开文件夹的替身调用这里，告诉访客要用桌面版
import i18n from "@app/i18n";

const MESSAGE: Record<string, string> = {
  "zh-CN": "网页版打不开你电脑上的文件和文件夹。桌面版可以。",
  "zh-TW": "網頁版打不開你電腦上的檔案和資料夾。桌面版可以。",
  en: "The web demo can't open files or folders on your computer. The desktop app can.",
  ja: "Web デモではパソコンのファイルやフォルダを開けません。デスクトップ版なら開けます。",
  "pt-BR": "A demo na web não abre arquivos nem pastas do seu computador. O app para desktop abre.",
  es: "La demo web no puede abrir archivos ni carpetas de tu equipo. La app de escritorio sí puede.",
};

/** Tells the visitor that this button works only in the desktop app. */
export function sayDesktopOnly(): void {
  const lng = (i18n.language || "").toLowerCase();
  const key =
    Object.keys(MESSAGE).find((k) => k.toLowerCase() === lng) ??
    Object.keys(MESSAGE).find((k) => lng.startsWith(k.slice(0, 2))) ??
    "en";
  window.alert(MESSAGE[key]);
}
