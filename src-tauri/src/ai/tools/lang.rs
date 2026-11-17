//! 聊天用的语言 `ChatLang`，以及工具结果和参数报错的多语言文字。
//! 聊天自己的文字在 [`crate::chat::lang`]。
//!
//! 为什么工具骨架也要本地化:模型"读"的资料若全是中文(头部/无命中提示/
//! 统计措辞),英文提问也会被证据语言拽回中文,本地小模型尤甚。因此所有
//! **模型可见**的骨架文本(系统提示词、工具结果头部、校验错误、降级文案)
//! 都按界面语言生成;证据卡等结构化数据不受影响。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatLang {
    ZhHans,
    ZhHant,
    En,
    Ja,
    Pt,
    Es,
}

impl ChatLang {
    /// BCP-47 宽松前缀匹配。None(旧前端没传)回落简中(历史行为不变);
    /// 认不出的标签回落英文(最通用)。
    pub fn from_tag(tag: Option<&str>) -> Self {
        let t = tag.unwrap_or("").trim().to_ascii_lowercase();
        if t.is_empty() {
            return Self::ZhHans;
        }
        if t.starts_with("zh") {
            if t.contains("tw") || t.contains("hk") || t.contains("mo") || t.contains("hant") {
                Self::ZhHant
            } else {
                Self::ZhHans
            }
        } else if t.starts_with("en") {
            Self::En
        } else if t.starts_with("ja") {
            Self::Ja
        } else if t.starts_with("pt") {
            Self::Pt
        } else if t.starts_with("es") {
            Self::Es
        } else {
            Self::En
        }
    }

    /// pub(crate):逐日分桶的结果行也要带本地化星期([`crate::ai::tools`])——
    /// 实测模型自己换算两周前是星期几会成列出错。
    pub(crate) fn weekday(self, iso: &str) -> &'static str {
        let i = iso.parse::<usize>().unwrap_or(7) - 1; // %u: 1=周一
        match self {
            Self::ZhHans => ["周一", "周二", "周三", "周四", "周五", "周六", "周日"][i],
            Self::ZhHant => ["週一", "週二", "週三", "週四", "週五", "週六", "週日"][i],
            Self::En => [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ][i],
            Self::Ja => [
                "月曜日",
                "火曜日",
                "水曜日",
                "木曜日",
                "金曜日",
                "土曜日",
                "日曜日",
            ][i],
            Self::Pt => [
                "segunda-feira",
                "terça-feira",
                "quarta-feira",
                "quinta-feira",
                "sexta-feira",
                "sábado",
                "domingo",
            ][i],
            Self::Es => [
                "lunes",
                "martes",
                "miércoles",
                "jueves",
                "viernes",
                "sábado",
                "domingo",
            ][i],
        }
    }

    // ── 工具结果骨架 ─────────────────────────────────────

    pub fn timeline_empty(self) -> &'static str {
        match self {
            Self::ZhHans => "该时段没有活动记录。",
            Self::ZhHant => "該時段沒有活動記錄。",
            Self::En => "No activity records in this period.",
            Self::Ja => "この期間には活動記録がありません。",
            Self::Pt => "Nenhum registro de atividade neste período.",
            Self::Es => "No hay registros de actividad en este periodo.",
        }
    }

    pub fn timeline_header_sampled(
        self,
        total: i64,
        first: &str,
        last: &str,
        shown: usize,
        per_hour: i64,
    ) -> String {
        match self {
            Self::ZhHans => format!(
                "该时段共 {total} 条活动记录,覆盖 {first} ~ {last}。以下为按小时抽样的 {shown} 条(每小时至多 {per_hour} 条,取时长最长;这是样本不是全量,时段总体结论以本行的总数与覆盖范围为准):"
            ),
            Self::ZhHant => format!(
                "該時段共 {total} 條活動記錄,涵蓋 {first} ~ {last}。以下為按小時抽樣的 {shown} 條(每小時至多 {per_hour} 條,取時長最長;這是樣本不是全量,時段整體結論以本行的總數與涵蓋範圍為準):"
            ),
            Self::En => format!(
                "{total} activity records in this period, spanning {first} ~ {last}. Below are {shown} sampled entries (up to {per_hour} per hour, longest first; this is a sample, not the full list — base period-level conclusions on the total and span in this line):"
            ),
            Self::Ja => format!(
                "この期間の活動記録は計 {total} 件({first} ~ {last})。以下は 1 時間あたり最大 {per_hour} 件(時間の長い順)で抽出した {shown} 件のサンプルです。全量ではないため、期間全体の結論はこの行の総数と範囲を基準にしてください:"
            ),
            Self::Pt => format!(
                "{total} registros de atividade neste período, abrangendo {first} ~ {last}. Abaixo, {shown} entradas amostradas (até {per_hour} por hora, maiores durações primeiro; é uma amostra, não a lista completa — baseie conclusões do período no total e na abrangência desta linha):"
            ),
            Self::Es => format!(
                "{total} registros de actividad en este periodo, que abarcan {first} ~ {last}. A continuación, {shown} entradas muestreadas (hasta {per_hour} por hora, las de mayor duración primero; es una muestra, no la lista completa — basa las conclusiones del periodo en el total y el alcance de esta línea):"
            ),
        }
    }

    pub fn timeline_header_all(self, total: i64) -> String {
        match self {
            Self::ZhHans => format!("该时段共 {total} 条活动记录,全部列出:"),
            Self::ZhHant => format!("該時段共 {total} 條活動記錄,全部列出:"),
            Self::En => format!("{total} activity records in this period, all listed:"),
            Self::Ja => format!("この期間の活動記録は計 {total} 件。すべて列挙します:"),
            Self::Pt => format!("{total} registros de atividade neste período, todos listados:"),
            Self::Es => format!("{total} registros de actividad en este periodo, todos listados:"),
        }
    }

    /// 覆盖披露行——拼在 timeline/search 结果最前。三个数字的口径:
    /// 活动日数来自主库(有没有用电脑),索引日数/待识别帧来自记忆库。
    /// "没搜到"究竟是"屏幕上没出现过"还是"索引不全",模型只有靠这行才能区分
    /// (措辞约束见 system_prompt 第 8 条)。
    pub fn coverage_line(self, activity_days: i64, covered_days: i64, pending: i64) -> String {
        if activity_days == 0 {
            return match self {
                Self::ZhHans => "覆盖情况:该范围内没有活动记录。".into(),
                Self::ZhHant => "覆蓋情況:該範圍內沒有活動記錄。".into(),
                Self::En => "Coverage: no activity records in this range.".into(),
                Self::Ja => "カバレッジ:この範囲に活動記録はありません。".into(),
                Self::Pt => "Cobertura: nenhum registro de atividade neste intervalo.".into(),
                Self::Es => "Cobertura: no hay registros de actividad en este intervalo.".into(),
            };
        }
        if covered_days == 0 && pending == 0 {
            // 一帧都没有:索引从未建立(未开截图/屏幕文字识别),与"识别中"分开表述
            return match self {
                Self::ZhHans => format!(
                    "覆盖情况:范围内 {activity_days} 个活动日均无屏幕文字索引(可能未开启截图或屏幕文字识别)。"
                ),
                Self::ZhHant => format!(
                    "覆蓋情況:範圍內 {activity_days} 個活動日均無螢幕文字索引(可能未開啟截圖或螢幕文字識別)。"
                ),
                Self::En => format!(
                    "Coverage: none of the {activity_days} active day(s) in this range have a screen-text index (screenshots or screen-text recognition may be off)."
                ),
                Self::Ja => format!(
                    "カバレッジ:範囲内の活動日 {activity_days} 日はいずれも画面テキスト索引がありません(スクリーンショットまたは画面テキスト認識が無効の可能性があります)。"
                ),
                Self::Pt => format!(
                    "Cobertura: nenhum dos {activity_days} dia(s) ativo(s) neste intervalo tem índice de texto de tela (capturas ou reconhecimento de texto podem estar desativados)."
                ),
                Self::Es => format!(
                    "Cobertura: ninguno de los {activity_days} día(s) activo(s) de este intervalo tiene índice de texto de pantalla (puede que las capturas o el reconocimiento de texto estén desactivados)."
                ),
            };
        }
        let base = match self {
            Self::ZhHans => format!(
                "覆盖情况:范围内 {activity_days} 个活动日中 {covered_days} 日有屏幕文字索引"
            ),
            Self::ZhHant => format!(
                "覆蓋情況:範圍內 {activity_days} 個活動日中 {covered_days} 日有螢幕文字索引"
            ),
            Self::En => format!(
                "Coverage: {covered_days} of {activity_days} active day(s) in this range have a screen-text index"
            ),
            Self::Ja => format!(
                "カバレッジ:範囲内の活動日 {activity_days} 日のうち {covered_days} 日に画面テキスト索引があります"
            ),
            Self::Pt => format!(
                "Cobertura: {covered_days} de {activity_days} dia(s) ativo(s) neste intervalo têm índice de texto de tela"
            ),
            Self::Es => format!(
                "Cobertura: {covered_days} de {activity_days} día(s) activo(s) de este intervalo tienen índice de texto de pantalla"
            ),
        };
        if pending > 0 {
            match self {
                Self::ZhHans => format!("{base},另有 {pending} 帧截图待识别。"),
                Self::ZhHant => format!("{base},另有 {pending} 幀截圖待識別。"),
                Self::En => format!("{base}, with {pending} frame(s) still awaiting recognition."),
                Self::Ja => format!("{base}。ほかに認識待ちのフレームが {pending} 件あります。"),
                Self::Pt => format!("{base}, com {pending} quadro(s) aguardando reconhecimento."),
                Self::Es => {
                    format!("{base}, y quedan {pending} fotograma(s) por reconocer.")
                }
            }
        } else {
            match self {
                Self::ZhHans | Self::ZhHant | Self::Ja => format!("{base}。"),
                Self::En | Self::Pt | Self::Es => format!("{base}."),
            }
        }
    }

    /// 搜索的窗口标题层小节头。标题命中 ≠ 屏幕文字命中:只说明该时段用过
    /// 相关窗口,没有屏幕内文字片段可引用——措辞里必须讲清这个差别。
    pub fn search_title_header(self, total: i64, shown: usize) -> String {
        if total as usize > shown {
            match self {
                Self::ZhHans => format!(
                    "窗口标题命中 {total} 条,显示最近 {shown} 条(标题匹配说明当时用过相关窗口,无屏幕内文字片段):"
                ),
                Self::ZhHant => format!(
                    "視窗標題命中 {total} 條,顯示最近 {shown} 條(標題匹配說明當時用過相關視窗,無螢幕內文字片段):"
                ),
                Self::En => format!(
                    "{total} window-title match(es); showing the {shown} most recent (a title match means the window was in use — no on-screen text snippet available):"
                ),
                Self::Ja => format!(
                    "ウィンドウタイトルのヒットは {total} 件。直近の {shown} 件を表示します(タイトル一致は当該ウィンドウを使っていたことを示すのみで、画面内テキストの抜粋はありません):"
                ),
                Self::Pt => format!(
                    "{total} correspondência(s) de título de janela; mostrando as {shown} mais recentes (uma correspondência de título indica que a janela estava em uso — sem trecho de texto da tela):"
                ),
                Self::Es => format!(
                    "{total} coincidencia(s) en títulos de ventana; se muestran las {shown} más recientes (una coincidencia de título solo indica que la ventana estaba en uso — sin fragmento de texto de la pantalla):"
                ),
            }
        } else {
            match self {
                Self::ZhHans => format!(
                    "窗口标题命中 {total} 条(标题匹配说明当时用过相关窗口,无屏幕内文字片段):"
                ),
                Self::ZhHant => format!(
                    "視窗標題命中 {total} 條(標題匹配說明當時用過相關視窗,無螢幕內文字片段):"
                ),
                Self::En => format!(
                    "{total} window-title match(es) (a title match means the window was in use — no on-screen text snippet available):"
                ),
                Self::Ja => format!(
                    "ウィンドウタイトルのヒットは {total} 件(タイトル一致は当該ウィンドウを使っていたことを示すのみで、画面内テキストの抜粋はありません):"
                ),
                Self::Pt => format!(
                    "{total} correspondência(s) de título de janela (uma correspondência de título indica que a janela estava em uso — sem trecho de texto da tela):"
                ),
                Self::Es => format!(
                    "{total} coincidencia(s) en títulos de ventana (una coincidencia de título solo indica que la ventana estaba en uso — sin fragmento de texto de la pantalla):"
                ),
            }
        }
    }

    pub fn search_no_hit(self) -> &'static str {
        match self {
            Self::ZhHans => "没有命中。可尝试换关键词(同义词/英文/更短的词)再搜。",
            Self::ZhHant => "沒有命中。可嘗試換關鍵字(同義詞/英文/更短的詞)再搜。",
            Self::En => "No hits. Try different keywords (synonyms, another language, or shorter terms).",
            Self::Ja => "ヒットしませんでした。別のキーワード(類義語/英語/より短い語)で再検索してください。",
            Self::Pt => "Nenhum resultado. Tente outras palavras-chave (sinônimos, outro idioma ou termos mais curtos).",
            Self::Es => "Ningún resultado. Prueba con otras palabras clave (sinónimos, otro idioma o términos más cortos).",
        }
    }

    pub fn search_header(self, total: i64, shown: usize) -> String {
        if total as usize > shown {
            match self {
                Self::ZhHans => format!(
                    "共 {total} 条命中,按相关度展示前 {shown} 条(需要更全可加日期范围收窄):"
                ),
                Self::ZhHant => format!(
                    "共 {total} 條命中,按相關度展示前 {shown} 條(需要更全可加日期範圍收窄):"
                ),
                Self::En => format!(
                    "{total} total hits; showing the top {shown} by relevance (narrow with a date range for better coverage):"
                ),
                Self::Ja => format!(
                    "計 {total} 件ヒット。関連度上位 {shown} 件を表示します(より網羅的に見るには日付範囲で絞り込んでください):"
                ),
                Self::Pt => format!(
                    "{total} resultados no total; mostrando os {shown} mais relevantes (restrinja com um intervalo de datas para mais cobertura):"
                ),
                Self::Es => format!(
                    "{total} resultados en total; se muestran los {shown} más relevantes (acota con un intervalo de fechas para tener más cobertura):"
                ),
            }
        } else {
            match self {
                Self::ZhHans => format!("共 {total} 条命中:"),
                Self::ZhHant => format!("共 {total} 條命中:"),
                Self::En => format!("{total} hits:"),
                Self::Ja => format!("計 {total} 件ヒット:"),
                Self::Pt => format!("{total} resultados:"),
                Self::Es => format!("{total} resultados:"),
            }
        }
    }

    pub fn stats_total(self, from: &str, to: &str, dur: &str) -> String {
        match self {
            Self::ZhHans => format!("{from} ~ {to} 合计: {dur}"),
            Self::ZhHant => format!("{from} ~ {to} 合計: {dur}"),
            Self::En => format!("{from} ~ {to} total: {dur}"),
            Self::Ja => format!("{from} ~ {to} 合計: {dur}"),
            Self::Pt => format!("{from} ~ {to} total: {dur}"),
            Self::Es => format!("{from} ~ {to} total: {dur}"),
        }
    }

    pub fn no_match(self, from: &str, to: &str) -> String {
        match self {
            Self::ZhHans => format!("{from} ~ {to} 无匹配记录"),
            Self::ZhHant => format!("{from} ~ {to} 無匹配記錄"),
            Self::En => format!("{from} ~ {to}: no matching records"),
            Self::Ja => format!("{from} ~ {to} 該当する記録はありません"),
            Self::Pt => format!("{from} ~ {to}: nenhum registro correspondente"),
            Self::Es => format!("{from} ~ {to}: ningún registro coincide"),
        }
    }

    pub fn duration_header(self, from: &str, to: &str, universe: i64, shown: usize) -> String {
        if universe as usize > shown {
            match self {
                Self::ZhHans => format!("{from} ~ {to} 共 {universe} 组,按时长取前 {shown} 组:"),
                Self::ZhHant => format!("{from} ~ {to} 共 {universe} 組,按時長取前 {shown} 組:"),
                Self::En => {
                    format!("{from} ~ {to}: {universe} groups total; top {shown} by duration:")
                }
                Self::Ja => {
                    format!("{from} ~ {to}: 全 {universe} グループ中、合計時間の上位 {shown} 件:")
                }
                Self::Pt => {
                    format!("{from} ~ {to}: {universe} grupos no total; top {shown} por duração:")
                }
                Self::Es => {
                    format!("{from} ~ {to}: {universe} grupos en total; top {shown} por duración:")
                }
            }
        } else {
            match self {
                Self::ZhHans => format!("{from} ~ {to} 按时长排序:"),
                Self::ZhHant => format!("{from} ~ {to} 按時長排序:"),
                Self::En => format!("{from} ~ {to}, sorted by duration:"),
                Self::Ja => format!("{from} ~ {to} 合計時間順:"),
                Self::Pt => format!("{from} ~ {to}, ordenado por duração:"),
                Self::Es => format!("{from} ~ {to}, ordenado por duración:"),
            }
        }
    }

    pub fn sessions_total(self, from: &str, to: &str, n: usize, gap: u32) -> String {
        match self {
            Self::ZhHans => format!("{from} ~ {to} 使用会话次数: {n} 次(以间隔≥{gap} 分钟算一次)"),
            Self::ZhHant => format!("{from} ~ {to} 使用會話次數: {n} 次(以間隔≥{gap} 分鐘算一次)"),
            Self::En => format!(
                "{from} ~ {to}: {n} usage sessions (a gap of ≥{gap} minutes starts a new session)"
            ),
            Self::Ja => format!(
                "{from} ~ {to} の使用セッション数: {n} 回({gap} 分以上の間隔で 1 回と数える)"
            ),
            Self::Pt => format!(
                "{from} ~ {to}: {n} sessões de uso (intervalo ≥{gap} minutos inicia nova sessão)"
            ),
            Self::Es => format!(
                "{from} ~ {to}: {n} sesiones de uso (un intervalo ≥{gap} minutos abre una sesión nueva)"
            ),
        }
    }

    pub fn sessions_grouped_header(
        self,
        from: &str,
        to: &str,
        universe: usize,
        shown: usize,
        gap: u32,
    ) -> String {
        let scope = universe > shown;
        match self {
            Self::ZhHans => {
                let s = if scope {
                    format!("共 {universe} 组,按次数取前 {shown} 组;")
                } else {
                    String::new()
                };
                format!("{from} ~ {to} 使用会话次数({s}以间隔≥{gap} 分钟算一次):")
            }
            Self::ZhHant => {
                let s = if scope {
                    format!("共 {universe} 組,按次數取前 {shown} 組;")
                } else {
                    String::new()
                };
                format!("{from} ~ {to} 使用會話次數({s}以間隔≥{gap} 分鐘算一次):")
            }
            Self::En => {
                let s = if scope {
                    format!("{universe} groups total, top {shown} by count; ")
                } else {
                    String::new()
                };
                format!(
                    "{from} ~ {to}: usage sessions ({s}a gap of ≥{gap} minutes starts a new session):"
                )
            }
            Self::Ja => {
                let s = if scope {
                    format!("全 {universe} グループ中、回数上位 {shown} 件;")
                } else {
                    String::new()
                };
                format!("{from} ~ {to} の使用セッション数({s}{gap} 分以上の間隔で 1 回と数える):")
            }
            Self::Pt => {
                let s = if scope {
                    format!("{universe} grupos no total, top {shown} por contagem; ")
                } else {
                    String::new()
                };
                format!(
                    "{from} ~ {to}: sessões de uso ({s}intervalo ≥{gap} minutos inicia nova sessão):"
                )
            }
            Self::Es => {
                let s = if scope {
                    format!("{universe} grupos en total, top {shown} por número; ")
                } else {
                    String::new()
                };
                format!(
                    "{from} ~ {to}: sesiones de uso ({s}un intervalo ≥{gap} minutos abre una sesión nueva):"
                )
            }
        }
    }

    /// 逐日分桶头部。`gap` = 会话计数口径的间隔分钟(时长口径传 None)。
    /// "无记录的日子未列出"必须写明:GROUP BY 只产出有活动的日子,
    /// 模型要知道缺行是零使用而不是数据缺失。
    pub fn bucket_day_header(self, from: &str, to: &str, n: usize, gap: Option<u32>) -> String {
        let g = self.gap_clause(gap);
        match self {
            Self::ZhHans => {
                format!("{from} ~ {to} 逐日统计({n} 个有记录的日子;无记录的日子未列出{g}):")
            }
            Self::ZhHant => {
                format!("{from} ~ {to} 逐日統計({n} 個有記錄的日子;無記錄的日子未列出{g}):")
            }
            Self::En => format!(
                "{from} ~ {to}, per-day breakdown ({n} day(s) with records; days without records are omitted{g}):"
            ),
            Self::Ja => {
                format!("{from} ~ {to} 日別集計(記録のある {n} 日。記録のない日は省略{g}):")
            }
            Self::Pt => format!(
                "{from} ~ {to}, detalhamento diário ({n} dia(s) com registros; dias sem registros omitidos{g}):"
            ),
            Self::Es => format!(
                "{from} ~ {to}, desglose diario ({n} día(s) con registros; los días sin registros se omiten{g}):"
            ),
        }
    }

    /// 逐周分桶头部。`auto` = 由逐日自动升级而来(范围超 60 天),要向模型说明
    /// 免得它以为工具没按参数执行。
    pub fn bucket_week_header(
        self,
        from: &str,
        to: &str,
        n: usize,
        auto: bool,
        gap: Option<u32>,
    ) -> String {
        let g = self.gap_clause(gap);
        let a = if auto {
            match self {
                Self::ZhHans => ";范围超过 60 天,已从逐日自动改为按周汇总",
                Self::ZhHant => ";範圍超過 60 天,已從逐日自動改為按週彙總",
                Self::En => "; the range exceeds 60 days, so per-day was switched to per-week automatically",
                Self::Ja => "。範囲が 60 日を超えたため日別から週別に自動変更",
                Self::Pt => "; o intervalo excede 60 dias, então o diário mudou automaticamente para semanal",
                Self::Es => "; el intervalo supera los 60 días, así que el desglose diario pasó automáticamente a semanal",
            }
        } else {
            ""
        };
        match self {
            Self::ZhHans => {
                format!("{from} ~ {to} 逐周统计({n} 周,每行日期为该周周一{a}{g}):")
            }
            Self::ZhHant => {
                format!("{from} ~ {to} 逐週統計({n} 週,每行日期為該週週一{a}{g}):")
            }
            Self::En => format!(
                "{from} ~ {to}, per-week breakdown ({n} week(s); each row is labeled by its Monday{a}{g}):"
            ),
            Self::Ja => {
                format!("{from} ~ {to} 週別集計({n} 週間。各行の日付はその週の月曜日{a}{g}):")
            }
            Self::Pt => format!(
                "{from} ~ {to}, detalhamento semanal ({n} semana(s); cada linha rotulada pela segunda-feira{a}{g}):"
            ),
            Self::Es => format!(
                "{from} ~ {to}, desglose semanal ({n} semana(s); cada fila lleva la etiqueta de su lunes{a}{g}):"
            ),
        }
    }

    /// 按一天 24 小时聚合的头部(作息分布)。
    pub fn bucket_hour_header(self, from: &str, to: &str, gap: Option<u32>) -> String {
        let g = self.gap_clause(gap);
        match self {
            Self::ZhHans => {
                format!("{from} ~ {to} 按一天中的小时聚合(范围内所有天的该小时累计{g}):")
            }
            Self::ZhHant => {
                format!("{from} ~ {to} 按一天中的小時聚合(範圍內所有天的該小時累計{g}):")
            }
            Self::En => format!(
                "{from} ~ {to}, aggregated by hour of day (each hour totals across all days in the range{g}):"
            ),
            Self::Ja => {
                format!("{from} ~ {to} 時間帯別集計(範囲内の全日について同じ時間帯を合算{g}):")
            }
            Self::Pt => format!(
                "{from} ~ {to}, agregado por hora do dia (cada hora soma todos os dias do intervalo{g}):"
            ),
            Self::Es => format!(
                "{from} ~ {to}, agregado por hora del día (cada hora suma todos los días del intervalo{g}):"
            ),
        }
    }

    /// 分桶头部里的会话间隔括注段(时长口径为空串)。
    fn gap_clause(self, gap: Option<u32>) -> String {
        let Some(gap) = gap else {
            return String::new();
        };
        match self {
            Self::ZhHans => format!(";会话以间隔≥{gap} 分钟算一次"),
            Self::ZhHant => format!(";會話以間隔≥{gap} 分鐘算一次"),
            Self::En => format!("; a gap of ≥{gap} minutes starts a new session"),
            Self::Ja => format!("。{gap} 分以上の間隔で 1 回と数える"),
            Self::Pt => format!("; intervalo ≥{gap} minutos inicia nova sessão"),
            Self::Es => format!("; un intervalo ≥{gap} minutos abre una sesión nueva"),
        }
    }

    /// first_last 口径的单行结论。
    pub fn first_last_line(self, from: &str, to: &str, first: &str, last: &str, n: i64) -> String {
        match self {
            Self::ZhHans => format!(
                "{from} ~ {to} 范围内共 {n} 条匹配活动:最早一次始于 {first},最近一次止于 {last}"
            ),
            Self::ZhHant => format!(
                "{from} ~ {to} 範圍內共 {n} 條匹配活動:最早一次始於 {first},最近一次止於 {last}"
            ),
            Self::En => format!(
                "{from} ~ {to}: {n} matching activities; the earliest starts at {first}, the latest ends at {last}"
            ),
            Self::Ja => format!(
                "{from} ~ {to} の一致する活動は {n} 件:最初は {first} に開始、直近は {last} に終了"
            ),
            Self::Pt => format!(
                "{from} ~ {to}: {n} atividades correspondentes; a mais antiga começa em {first}, a mais recente termina em {last}"
            ),
            Self::Es => format!(
                "{from} ~ {to}: {n} actividades coincidentes; la más antigua empieza a las {first} y la más reciente termina a las {last}"
            ),
        }
    }

    pub fn count_suffix(self, n: usize) -> String {
        match self {
            Self::ZhHans => format!("{n} 次"),
            Self::ZhHant => format!("{n} 次"),
            Self::En => format!("{n} sessions"),
            Self::Ja => format!("{n} 回"),
            Self::Pt => format!("{n} sessões"),
            Self::Es => format!("{n} sesiones"),
        }
    }

    pub fn fmt_secs(self, secs: i64) -> String {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        match self {
            Self::ZhHans => {
                if h > 0 {
                    format!("{h} 小时 {m} 分钟")
                } else {
                    format!("{m} 分钟")
                }
            }
            Self::ZhHant => {
                if h > 0 {
                    format!("{h} 小時 {m} 分鐘")
                } else {
                    format!("{m} 分鐘")
                }
            }
            Self::En => {
                if h > 0 {
                    format!("{h} hr {m} min")
                } else {
                    format!("{m} min")
                }
            }
            Self::Ja => {
                if h > 0 {
                    format!("{h}時間{m}分")
                } else {
                    format!("{m}分")
                }
            }
            Self::Pt => {
                if h > 0 {
                    format!("{h} h {m} min")
                } else {
                    format!("{m} min")
                }
            }
            Self::Es => {
                if h > 0 {
                    format!("{h} h {m} min")
                } else {
                    format!("{m} min")
                }
            }
        }
    }

    // ── 参数校验错误(回填给模型自纠) ────────────────────

    pub fn err_unknown_tool(self, other: &str) -> String {
        match self {
            Self::ZhHans => format!("未知工具 {other},只能用 search_text / query_stats / get_timeline"),
            Self::ZhHant => format!("未知工具 {other},只能用 search_text / query_stats / get_timeline"),
            Self::En => format!("Unknown tool {other}; only search_text / query_stats / get_timeline are available"),
            Self::Ja => format!("不明なツール {other}。search_text / query_stats / get_timeline のみ使用できます"),
            Self::Pt => format!("Ferramenta desconhecida {other}; apenas search_text / query_stats / get_timeline estão disponíveis"),
            Self::Es => format!("Herramienta desconocida {other}; solo están disponibles search_text / query_stats / get_timeline"),
        }
    }

    pub fn err_need_range(self, tool: &str) -> String {
        match self {
            Self::ZhHans => format!("{tool} 需要 date_from 和 date_to(YYYY-MM-DD)"),
            Self::ZhHant => format!("{tool} 需要 date_from 和 date_to(YYYY-MM-DD)"),
            Self::En => format!("{tool} requires date_from and date_to (YYYY-MM-DD)"),
            Self::Ja => format!("{tool} には date_from と date_to(YYYY-MM-DD)が必要です"),
            Self::Pt => format!("{tool} requer date_from e date_to (YYYY-MM-DD)"),
            Self::Es => format!("{tool} requiere date_from y date_to (YYYY-MM-DD)"),
        }
    }

    pub fn err_bad_date(self, field: &str, val: &str) -> String {
        match self {
            Self::ZhHans => format!("{field} 不是有效日期: {val}"),
            Self::ZhHant => format!("{field} 不是有效日期: {val}"),
            Self::En => format!("{field} is not a valid date: {val}"),
            Self::Ja => format!("{field} は有効な日付ではありません: {val}"),
            Self::Pt => format!("{field} não é uma data válida: {val}"),
            Self::Es => format!("{field} no es una fecha válida: {val}"),
        }
    }

    pub fn err_from_after_to(self) -> &'static str {
        match self {
            Self::ZhHans => "date_from 晚于 date_to",
            Self::ZhHant => "date_from 晚於 date_to",
            Self::En => "date_from is later than date_to",
            Self::Ja => "date_from が date_to より後になっています",
            Self::Pt => "date_from é posterior a date_to",
            Self::Es => "date_from es posterior a date_to",
        }
    }

    pub fn err_range_too_long(self) -> &'static str {
        match self {
            Self::ZhHans => "时间跨度超过 366 天,请缩小范围",
            Self::ZhHant => "時間跨度超過 366 天,請縮小範圍",
            Self::En => "The range exceeds 366 days; please narrow it",
            Self::Ja => "期間が 366 日を超えています。範囲を狭めてください",
            Self::Pt => "O intervalo excede 366 dias; reduza-o",
            Self::Es => "El intervalo supera los 366 días; redúcelo",
        }
    }

    pub fn err_from_in_future(self) -> &'static str {
        match self {
            Self::ZhHans => "date_from 在未来",
            Self::ZhHant => "date_from 在未來",
            Self::En => "date_from is in the future",
            Self::Ja => "date_from が未来の日付です",
            Self::Pt => "date_from está no futuro",
            Self::Es => "date_from está en el futuro",
        }
    }

    pub fn err_keywords_empty(self) -> &'static str {
        match self {
            Self::ZhHans => "keywords 不能为空",
            Self::ZhHant => "keywords 不能為空",
            Self::En => "keywords must not be empty",
            Self::Ja => "keywords を空にはできません",
            Self::Pt => "keywords não pode estar vazio",
            Self::Es => "keywords no puede estar vacío",
        }
    }

    pub fn err_item_too_long(self, field: &str) -> String {
        match self {
            Self::ZhHans => format!("{field} 里有超过 64 字符的项"),
            Self::ZhHant => format!("{field} 裡有超過 64 字元的項"),
            Self::En => format!("{field} contains an item longer than 64 characters"),
            Self::Ja => format!("{field} に 64 文字を超える項目があります"),
            Self::Pt => format!("{field} contém um item com mais de 64 caracteres"),
            Self::Es => format!("{field} contiene un elemento de más de 64 caracteres"),
        }
    }

    pub fn err_title_kw_too_long(self) -> &'static str {
        match self {
            Self::ZhHans => "title_keyword 过长(≤64 字符)",
            Self::ZhHant => "title_keyword 過長(≤64 字元)",
            Self::En => "title_keyword is too long (max 64 characters)",
            Self::Ja => "title_keyword が長すぎます(64 文字以内)",
            Self::Pt => "title_keyword é longo demais (máx. 64 caracteres)",
            Self::Es => "title_keyword es demasiado largo (máx. 64 caracteres)",
        }
    }

    pub fn err_bucket_with_group(self) -> &'static str {
        match self {
            Self::ZhHans => "bucket 与 group_by 不能同时使用:看趋势用 bucket,看排行用 group_by,请去掉其一",
            Self::ZhHant => "bucket 與 group_by 不能同時使用:看趨勢用 bucket,看排行用 group_by,請去掉其一",
            Self::En => "bucket and group_by cannot be combined: use bucket for trends, group_by for rankings — drop one of them",
            Self::Ja => "bucket と group_by は併用できません。推移は bucket、ランキングは group_by を使い、どちらか一方を外してください",
            Self::Pt => "bucket e group_by não podem ser combinados: use bucket para tendências, group_by para rankings — remova um deles",
            Self::Es => "bucket y group_by no se pueden combinar: usa bucket para tendencias y group_by para rankings — quita uno de los dos",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 六语 × 全部格式化方法的批量烟测:每个变体都产出非空文案,
    /// 且插值参数(日期/数字/工具名)确实出现在结果里——防"漏改某语言"和
    /// "占位符没插进去"两类低级错误,一次覆盖全部 match 臂。
    #[test]
    fn all_langs_produce_nonempty_interpolated_strings() {
        const LANGS: [ChatLang; 6] = [
            ChatLang::ZhHans,
            ChatLang::ZhHant,
            ChatLang::En,
            ChatLang::Ja,
            ChatLang::Pt,
            ChatLang::Es,
        ];
        for lang in LANGS {
            // 静态文案:非空即可
            for s in [
                lang.timeline_empty(),
                lang.search_no_hit(),
                lang.err_from_after_to(),
                lang.err_range_too_long(),
                lang.err_from_in_future(),
                lang.err_keywords_empty(),
                lang.err_title_kw_too_long(),
                lang.err_bucket_with_group(),
            ] {
                assert!(!s.is_empty(), "{lang:?} 有空文案");
            }
            // 七天星期全枚举(越界回退不 panic 由 weekday 内部保证,此处只走合法值)
            for d in 1..=7 {
                assert!(!lang.weekday(&d.to_string()).is_empty());
            }
            // 插值类:参数必须真的出现在产物里
            assert!(lang.err_unknown_tool("nope").contains("nope"));
            assert!(lang.err_need_range("query_stats").contains("query_stats"));
            assert!(lang
                .err_bad_date("date_from", "26-07")
                .contains("date_from"));
            assert!(lang.err_item_too_long("apps").contains("apps"));
            let h = lang.timeline_header_sampled(120, "2026-07-01", "2026-07-07", 40, 7);
            assert!(h.contains("120") && h.contains("40"), "{lang:?}: {h}");
            assert!(lang.timeline_header_all(9).contains('9'));
            let cov = lang.coverage_line(30, 12, 3);
            assert!(cov.contains("12") && cov.contains('3'), "{lang:?}: {cov}");
            assert!(lang.search_title_header(88, 10).contains("88"));
            assert!(lang.search_header(77, 5).contains("77"));
            assert!(lang
                .stats_total("2026-07-01", "2026-07-07", "1h")
                .contains("1h"));
            assert!(lang
                .no_match("2026-07-01", "2026-07-07")
                .contains("2026-07-01"));
            let dh = lang.duration_header("2026-07-01", "2026-07-07", 12, 5);
            assert!(dh.contains("12"), "{lang:?}: {dh}");
            let st = lang.sessions_total("2026-07-01", "2026-07-07", 4, 30);
            assert!(st.contains('4') && st.contains("30"), "{lang:?}: {st}");
            let sg = lang.sessions_grouped_header("2026-07-01", "2026-07-07", 8, 5, 30);
            assert!(sg.contains('8'), "{lang:?}: {sg}");
            // 分桶头部:计数 + gap 括注两种形态
            let bd = lang.bucket_day_header("2026-07-01", "2026-07-07", 6, None);
            assert!(bd.contains('6'), "{lang:?}: {bd}");
            let bdg = lang.bucket_day_header("2026-07-01", "2026-07-07", 6, Some(30));
            assert!(bdg.contains("30"), "{lang:?}: {bdg}");
            let bw = lang.bucket_week_header("2026-06-01", "2026-07-07", 5, true, None);
            assert!(
                bw.contains('5')
                    && bw.len()
                        > lang
                            .bucket_week_header("2026-06-01", "2026-07-07", 5, false, None)
                            .len(),
                "{lang:?}:自动升周说明缺失"
            );
            assert!(!lang
                .bucket_hour_header("2026-07-01", "2026-07-07", Some(15))
                .is_empty());
            let fl = lang.first_last_line(
                "2026-07-01",
                "2026-07-07",
                "2026-07-02 09:00",
                "2026-07-06 21:00",
                42,
            );
            assert!(
                fl.contains("42") && fl.contains("2026-07-02 09:00"),
                "{lang:?}: {fl}"
            );
            assert!(lang.count_suffix(3).contains('3'));
            // fmt_secs 的三个量级形态
            assert!(!lang.fmt_secs(59).is_empty());
            assert!(!lang.fmt_secs(61 * 60).is_empty());
            assert!(!lang.fmt_secs(3 * 3600 + 5 * 60).is_empty());
        }
    }

    #[test]
    fn tag_parsing_prefix_and_fallbacks() {
        assert_eq!(ChatLang::from_tag(Some("zh-CN")), ChatLang::ZhHans);
        assert_eq!(ChatLang::from_tag(Some("zh-TW")), ChatLang::ZhHant);
        assert_eq!(ChatLang::from_tag(Some("zh-Hant-HK")), ChatLang::ZhHant);
        assert_eq!(ChatLang::from_tag(Some("en-US")), ChatLang::En);
        assert_eq!(ChatLang::from_tag(Some("ja")), ChatLang::Ja);
        assert_eq!(ChatLang::from_tag(Some("pt-BR")), ChatLang::Pt);
        assert_eq!(ChatLang::from_tag(Some("es")), ChatLang::Es);
        assert_eq!(ChatLang::from_tag(Some("es-MX")), ChatLang::Es);
        // 旧前端没传 → 维持历史行为(简中);认不出的 → 英文
        assert_eq!(ChatLang::from_tag(None), ChatLang::ZhHans);
        assert_eq!(ChatLang::from_tag(Some("fr")), ChatLang::En);
    }
}
