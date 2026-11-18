//! 聊天用的语言 `ChatLang`，以及时间线、用量两个工具的多语言文字。
//! 别的工具文字和共用的句子在 JSON 里，读法见 [`super::text`]；
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

    // ── 用量工具的参数报错 ────────────────────────────

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
                lang.err_title_kw_too_long(),
                lang.err_bucket_with_group(),
            ] {
                assert!(!s.is_empty(), "{lang:?} 有空文案");
            }
            // 插值类:参数必须真的出现在产物里
            let h = lang.timeline_header_sampled(120, "2026-07-01", "2026-07-07", 40, 7);
            assert!(h.contains("120") && h.contains("40"), "{lang:?}: {h}");
            assert!(lang.timeline_header_all(9).contains('9'));
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
