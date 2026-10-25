// 主题页的语言和文字。只有这一页用，不放进应用的语言文件。

export type PageLanguage = "zh-CN" | "zh-TW" | "en" | "ja" | "pt-BR" | "es";

const LANGUAGES: readonly string[] = ["zh-CN", "zh-TW", "en", "ja", "pt-BR", "es"];

/**
 * 页面用的语言：先看 ?lang=（应用里的「获取主题」按钮会带上界面语言），再看浏览器的语言，
 * 都不认识就用英文。
 */
export function pageLanguage(search: string, browserLanguages: readonly string[]): PageLanguage {
  const asked = new URLSearchParams(search).get("lang");
  if (asked !== null && LANGUAGES.includes(asked)) return asked as PageLanguage;
  for (const tag of browserLanguages) {
    const language = fromBrowserTag(tag);
    if (language !== null) return language;
  }
  return "en";
}

/** 浏览器的语言标签（zh-HK、pt-PT、es-419……）对到页面的语言。 */
function fromBrowserTag(tag: string): PageLanguage | null {
  const t = tag.toLowerCase();
  if (t === "zh-tw" || t === "zh-hk" || t === "zh-mo" || t.startsWith("zh-hant")) return "zh-TW";
  if (t.startsWith("zh")) return "zh-CN";
  if (t.startsWith("ja")) return "ja";
  if (t.startsWith("pt")) return "pt-BR";
  if (t.startsWith("es")) return "es";
  if (t.startsWith("en")) return "en";
  return null;
}

/** 这种语言的官网首页。西班牙语还没有首页，用英文的。 */
export const HOME_PAGE: Record<PageLanguage, string> = {
  "zh-CN": "/",
  "zh-TW": "/tw/",
  en: "/en/",
  ja: "/ja/",
  "pt-BR": "/pt/",
  es: "/en/",
};

export interface Strings {
  title: string;
  intro: string;
  install: string;
  loading: string;
  empty: string;
  makeOne: string;
  loadFailed: (reason: string) => string;
  retry: string;
  preview: string;
  homepage: string;
  download: string;
  downloads: (count: string) => string;
  close: string;
}

// 「设置 - 外观 - 自定义主题」「导入主题」照应用界面上的叫法
export const STRINGS: Record<PageLanguage, Strings> = {
  "zh-CN": {
    title: "主题",
    intro: "给 Hindsight 换个样子。每个主题都能先在这里试用。",
    install: "下载后解压，在 Hindsight 的「设置 - 外观 - 自定义主题」里点「导入主题」，选 theme.json。",
    loading: "正在读取主题…",
    empty: "还没有主题。",
    makeOne: "做一个主题",
    loadFailed: (reason) => `读不到主题列表：${reason}`,
    retry: "重试",
    preview: "预览",
    homepage: "主页",
    download: "下载",
    downloads: (count) => `${count} 次下载`,
    close: "关闭",
  },
  "zh-TW": {
    title: "主題",
    intro: "幫 Hindsight 換個樣子。每個主題都能先在這裡試用。",
    install: "下載後解壓縮，在 Hindsight 的「設定 - 外觀 - 自訂主題」裡點「匯入主題」，選 theme.json。",
    loading: "正在讀取主題…",
    empty: "還沒有主題。",
    makeOne: "做一個主題",
    loadFailed: (reason) => `讀不到主題列表：${reason}`,
    retry: "重試",
    preview: "預覽",
    homepage: "主頁",
    download: "下載",
    downloads: (count) => `${count} 次下載`,
    close: "關閉",
  },
  en: {
    title: "Themes",
    intro: "Give Hindsight a new look. You can try each theme here first.",
    install:
      'Download and unzip it. In Hindsight, open Settings - Appearance - Custom themes, click "Import theme" and choose theme.json.',
    loading: "Reading the themes…",
    empty: "There are no themes yet.",
    makeOne: "Make a theme",
    loadFailed: (reason) => `Cannot read the theme list: ${reason}`,
    retry: "Try again",
    preview: "Preview",
    homepage: "Homepage",
    download: "Download",
    downloads: (count) => `${count} downloads`,
    close: "Close",
  },
  ja: {
    title: "テーマ",
    intro: "Hindsight の見た目を変えましょう。どのテーマもここで試せます。",
    install:
      "ダウンロードして展開し、Hindsight の「設定 - 外観 - カスタムテーマ」で「テーマを読み込む」をクリックして theme.json を選びます。",
    loading: "テーマを読み込み中…",
    empty: "まだテーマはありません。",
    makeOne: "テーマを作る",
    loadFailed: (reason) => `テーマの一覧を読み込めません：${reason}`,
    retry: "再試行",
    preview: "プレビュー",
    homepage: "ホームページ",
    download: "ダウンロード",
    downloads: (count) => `${count} 回ダウンロード`,
    close: "閉じる",
  },
  "pt-BR": {
    title: "Temas",
    intro: "Dê um novo visual ao Hindsight. Dá para testar cada tema aqui antes.",
    install:
      'Baixe e descompacte. No Hindsight, abra Ajustes - Aparência - Temas personalizados, clique em "Importar tema" e escolha theme.json.',
    loading: "Lendo os temas…",
    empty: "Ainda não há temas.",
    makeOne: "Criar um tema",
    loadFailed: (reason) => `Não foi possível ler a lista de temas: ${reason}`,
    retry: "Tentar de novo",
    preview: "Visualizar",
    homepage: "Página",
    download: "Baixar",
    downloads: (count) => `${count} downloads`,
    close: "Fechar",
  },
  es: {
    title: "Temas",
    intro: "Dale a Hindsight un aspecto nuevo. Puedes probar cada tema aquí antes.",
    install:
      "Descárgalo y descomprímelo. En Hindsight, abre Ajustes - Apariencia - Temas personalizados, haz clic en «Importar tema» y elige theme.json.",
    loading: "Leyendo los temas…",
    empty: "Todavía no hay temas.",
    makeOne: "Crear un tema",
    loadFailed: (reason) => `No se pudo leer la lista de temas: ${reason}`,
    retry: "Reintentar",
    preview: "Vista previa",
    homepage: "Página",
    download: "Descargar",
    downloads: (count) => `${count} descargas`,
    close: "Cerrar",
  },
};
