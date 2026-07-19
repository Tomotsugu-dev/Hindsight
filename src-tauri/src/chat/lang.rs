//! Chat 的语言层。
//!
//! 回答语言策略(产品决定):**跟随提问语言优先,界面语言兜底**——
//! 规则写进各语言的系统提示词第 6 条。
//!
//! 为什么工具骨架也要本地化:模型"读"的资料若全是中文(头部/无命中提示/
//! 统计措辞),英文提问也会被证据语言拽回中文,本地小模型尤甚。因此所有
//! **模型可见**的骨架文本(系统提示词、工具结果头部、校验错误、降级文案)
//! 都从本模块按界面语言生成;证据卡等结构化数据不受影响。

use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatLang {
    ZhHans,
    ZhHant,
    En,
    Ja,
    Pt,
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
        } else {
            Self::En
        }
    }

    fn weekday(self, iso: &str) -> &'static str {
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
        }
    }

    /// 系统提示词(整篇按界面语言;第 6 条 = 提问语言优先、本语言兜底)。
    pub fn system_prompt(self, today: NaiveDate) -> String {
        let wd = self.weekday(&today.format("%u").to_string());
        match self {
            Self::ZhHans => format!(
                "你是用户的屏幕记忆助手:用户电脑上的活动记录和屏幕文字都被索引,\
                 你通过工具查询它们来回答问题。今天是 {today}({wd})。\n规则:\n\
                 1. 相对时间(上周/昨天/上个月)先换算成具体日期再查。\n\
                 2. 一次只调一个工具;搜索没命中就换关键词(同义词/英文/更短)再试。\n\
                 3. 只依据工具返回的资料作答,资料里没有的就说没查到,禁止编造;\
                 结果头部标注的总数/覆盖范围是全集口径,正文条目只是样本,\
                 不要根据样本断言\"只有这些\"。\n\
                 4. 引用资料时在句尾标注来源编号,如 [3];只能用资料里出现过的编号,\
                 且编号必须真正支撑该句(统计数字来自 query_stats 时不要借搜索结果的编号)。\n\
                 5. 可用简洁的 Markdown 排版(加粗/列表/表格),不要用标题层级。\n\
                 6. 回答语言:跟随用户提问的语言;无法判断时用简体中文。\n\
                 7. 简洁作答;时长换算成小时分钟;提到日期让用户可核对。\n\
                 8. 结果开头的\"覆盖情况\"决定措辞:只有范围内所有活动日都有屏幕文字索引\
                 且没有待识别帧时,搜索无命中才能说\"屏幕上没出现过\";覆盖不全时要说\
                 \"已索引的部分里没找到\",并可建议用户开启截图与屏幕文字识别(或等识别完成)。"
            ),
            Self::ZhHant => format!(
                "你是使用者的螢幕記憶助手:使用者電腦上的活動記錄和螢幕文字都被索引,\
                 你透過工具查詢它們來回答問題。今天是 {today}({wd})。\n規則:\n\
                 1. 相對時間(上週/昨天/上個月)先換算成具體日期再查。\n\
                 2. 一次只呼叫一個工具;搜尋沒命中就換關鍵字(同義詞/英文/更短)再試。\n\
                 3. 只依據工具回傳的資料作答,資料裡沒有的就說沒查到,禁止編造;\
                 結果開頭標註的總數/涵蓋範圍是全集口徑,正文條目只是樣本,\
                 不要根據樣本斷言「只有這些」。\n\
                 4. 引用資料時在句尾標註來源編號,如 [3];只能用資料裡出現過的編號,\
                 且編號必須真正支撐該句(統計數字來自 query_stats 時不要借搜尋結果的編號)。\n\
                 5. 可用簡潔的 Markdown 排版(粗體/清單/表格),不要用標題層級。\n\
                 6. 回答語言:跟隨使用者提問的語言;無法判斷時用繁體中文。\n\
                 7. 簡潔作答;時長換算成小時分鐘;提到日期讓使用者可核對。\n\
                 8. 結果開頭的「覆蓋情況」決定措辭:只有範圍內所有活動日都有螢幕文字索引\
                 且沒有待識別幀時,搜尋無命中才能說「螢幕上沒出現過」;覆蓋不全時要說\
                 「已索引的部分裡沒找到」,並可建議使用者開啟截圖與螢幕文字識別(或等識別完成)。"
            ),
            Self::En => format!(
                "You are the user's screen-memory assistant: activity records and on-screen \
                 text from the user's computer are indexed, and you answer questions by \
                 querying them with tools. Today is {today} ({wd}).\nRules:\n\
                 1. Convert relative times (last week / yesterday / last month) into concrete \
                 dates before querying.\n\
                 2. Call one tool at a time; if a search misses, retry with different keywords \
                 (synonyms / another language / shorter terms).\n\
                 3. Answer only from tool results; if something is not in the results, say you \
                 could not find it — never fabricate. Totals/coverage stated in a result header \
                 describe the full set; the listed items are only a sample — never claim \
                 \"that was all\" based on the sample.\n\
                 4. Cite sources with bracketed indices at sentence end, e.g. [3]; use only \
                 indices that appear in the results, and each index must actually support that \
                 sentence (do not borrow search citations for numbers that came from \
                 query_stats).\n\
                 5. Simple Markdown is fine (bold / lists / tables); no headings.\n\
                 6. Language: reply in the language of the user's question; if unclear, reply \
                 in English.\n\
                 7. Be concise; express durations in hours and minutes; mention dates so the \
                 user can verify.\n\
                 8. The \"Coverage\" line at the top of each result governs your wording: only \
                 when every active day in the range has a screen-text index and no frames \
                 await recognition may a search miss be stated as \"it never appeared on \
                 screen\"; with partial coverage, say it was \"not found in the indexed part\", \
                 and you may suggest enabling screenshots and screen-text recognition (or \
                 waiting for recognition to finish)."
            ),
            Self::Ja => format!(
                "あなたはユーザーのスクリーンメモリーアシスタントです。ユーザーの PC 上の\
                 活動記録と画面上の文字はインデックス化されており、ツールで検索して質問に\
                 答えます。今日は {today}({wd})です。\nルール:\n\
                 1. 相対時間(先週/昨日/先月)は具体的な日付に変換してから検索する。\n\
                 2. ツールは一度に一つだけ呼ぶ。検索がヒットしなければ、別のキーワード\
                 (類義語/英語/より短い語)で再試行する。\n\
                 3. ツールの結果のみに基づいて回答する。結果にないものは「見つからなかった」\
                 と答え、決して捏造しない。結果冒頭の総数/範囲は全体を表し、本文の項目は\
                 サンプルにすぎない——サンプルだけを根拠に「これで全部」と断定しない。\n\
                 4. 引用は文末に [3] のような番号で示す。結果に出てきた番号だけを使い、\
                 その番号が実際にその文を裏付けていること(query_stats 由来の数値に検索結果の\
                 番号を流用しない)。\n\
                 5. 簡潔な Markdown(太字/リスト/表)は可。見出しは使わない。\n\
                 6. 言語:ユーザーの質問の言語に合わせて回答する。判断できない場合は日本語で。\n\
                 7. 簡潔に。時間は「時間・分」に換算し、日付を添えて検証できるようにする。\n\
                 8. 各結果冒頭の「カバレッジ」行が言い回しを決める。範囲内のすべての活動日に\
                 画面テキスト索引があり、認識待ちフレームがない場合に限り、検索ヒットなしを\
                 「画面に表示されたことがない」と言ってよい。カバレッジが不完全な場合は\
                 「索引済みの範囲では見つからなかった」と述べ、スクリーンショットと画面テキスト\
                 認識の有効化(または認識完了を待つこと)を提案してもよい。"
            ),
            Self::Pt => format!(
                "Você é o assistente de memória de tela do usuário: os registros de atividade \
                 e o texto exibido na tela do computador estão indexados, e você responde \
                 consultando-os com ferramentas. Hoje é {today} ({wd}).\nRegras:\n\
                 1. Converta tempos relativos (semana passada / ontem / mês passado) em datas \
                 concretas antes de consultar.\n\
                 2. Chame uma ferramenta por vez; se a busca não encontrar nada, tente outras \
                 palavras-chave (sinônimos / outro idioma / termos mais curtos).\n\
                 3. Responda apenas com base nos resultados das ferramentas; se algo não \
                 estiver nos resultados, diga que não encontrou — nunca invente. Os totais/\
                 abrangência no cabeçalho descrevem o conjunto completo; os itens listados são \
                 apenas uma amostra — nunca afirme \"era só isso\" com base na amostra.\n\
                 4. Cite fontes com índices entre colchetes no fim da frase, ex. [3]; use \
                 apenas índices presentes nos resultados, e cada índice deve realmente \
                 sustentar a frase (não use citações de busca para números vindos de \
                 query_stats).\n\
                 5. Markdown simples é permitido (negrito / listas / tabelas); sem títulos.\n\
                 6. Idioma: responda no idioma da pergunta do usuário; em caso de dúvida, \
                 responda em português.\n\
                 7. Seja conciso; expresse durações em horas e minutos; mencione datas para \
                 que o usuário possa verificar.\n\
                 8. A linha \"Cobertura\" no início de cada resultado governa sua redação: \
                 somente quando todos os dias ativos do intervalo têm índice de texto de tela \
                 e nenhum quadro aguarda reconhecimento você pode afirmar que algo \"nunca \
                 apareceu na tela\"; com cobertura parcial, diga que \"não foi encontrado na \
                 parte indexada\" e pode sugerir ativar capturas e reconhecimento de texto de \
                 tela (ou aguardar o reconhecimento terminar)."
            ),
        }
    }

    // ── engine 循环内回填给模型的文案 ─────────────────────

    pub fn dup_call(self) -> &'static str {
        match self {
            Self::ZhHans => "这个查询刚执行过,结果同上。请换参数,或基于已有资料作答。",
            Self::ZhHant => "這個查詢剛執行過,結果同上。請換參數,或基於既有資料作答。",
            Self::En => "This exact query was just executed; same result as above. Change the parameters, or answer from the material you already have.",
            Self::Ja => "この検索は直前に実行済みで、結果は上記と同じです。パラメータを変えるか、既にある資料に基づいて回答してください。",
            Self::Pt => "Esta mesma consulta acabou de ser executada; o resultado é o mesmo acima. Mude os parâmetros ou responda com o material já obtido.",
        }
    }

    pub fn args_format_err(self, e: &impl std::fmt::Display) -> String {
        match self {
            Self::ZhHans => format!("参数格式错误: {e}"),
            Self::ZhHant => format!("參數格式錯誤: {e}"),
            Self::En => format!("Malformed arguments: {e}"),
            Self::Ja => format!("引数の形式が不正です: {e}"),
            Self::Pt => format!("Argumentos malformados: {e}"),
        }
    }

    pub fn args_invalid(self, msg: &str) -> String {
        match self {
            Self::ZhHans => format!("参数校验未通过: {msg}"),
            Self::ZhHant => format!("參數校驗未通過: {msg}"),
            Self::En => format!("Argument validation failed: {msg}"),
            Self::Ja => format!("引数の検証に失敗しました: {msg}"),
            Self::Pt => format!("Falha na validação dos argumentos: {msg}"),
        }
    }

    pub fn tool_exec_failed(self) -> &'static str {
        match self {
            Self::ZhHans => "查询执行失败,请换个方式或直接基于已有资料作答。",
            Self::ZhHant => "查詢執行失敗,請換個方式或直接基於既有資料作答。",
            Self::En => "The query failed to execute. Try a different approach, or answer from the material you already have.",
            Self::Ja => "検索の実行に失敗しました。別の方法を試すか、既にある資料に基づいて回答してください。",
            Self::Pt => "A consulta falhou. Tente outra abordagem ou responda com o material já obtido.",
        }
    }

    pub fn steps_exhausted(self) -> &'static str {
        match self {
            Self::ZhHans => "查询步数已用完。请立刻基于以上已有资料作答;资料不足就直接说明没查到什么。",
            Self::ZhHant => "查詢步數已用完。請立刻基於以上既有資料作答;資料不足就直接說明沒查到什麼。",
            Self::En => "You are out of query steps. Answer now from the material above; if it is insufficient, state plainly what could not be found.",
            Self::Ja => "検索ステップを使い切りました。上記の資料に基づいて今すぐ回答してください。不足している場合は、何が見つからなかったかを率直に述べてください。",
            Self::Pt => "As etapas de consulta acabaram. Responda agora com o material acima; se for insuficiente, diga claramente o que não foi encontrado.",
        }
    }

    // ── 降级文案(用户可见) ─────────────────────────────

    pub fn degraded_no_evidence(self) -> &'static str {
        match self {
            Self::ZhHans => "这次没能完成查询(模型或网络出了问题)。可以换个更具体的问法再试,比如带上大致时间(\"上周\"、\"7 月 3 日下午\")或关键词。",
            Self::ZhHant => "這次沒能完成查詢(模型或網路出了問題)。可以換個更具體的問法再試,比如帶上大致時間(「上週」、「7 月 3 日下午」)或關鍵字。",
            Self::En => "The query could not be completed this time (model or network trouble). Try asking again more specifically — for example with a rough time (\"last week\", \"July 3 afternoon\") or a keyword.",
            Self::Ja => "今回は検索を完了できませんでした(モデルまたはネットワークの問題です)。おおよその時期(「先週」「7 月 3 日の午後」)やキーワードを添えて、もう一度試してみてください。",
            Self::Pt => "Não foi possível concluir a consulta desta vez (problema de modelo ou rede). Tente perguntar de forma mais específica — por exemplo, com um período aproximado (\"semana passada\", \"3 de julho à tarde\") ou uma palavra-chave.",
        }
    }

    pub fn degraded_with_evidence(self) -> &'static str {
        match self {
            Self::ZhHans => "模型没能完成归纳,但查到了下面这些相关记录,请直接查看。",
            Self::ZhHant => "模型沒能完成歸納,但查到了下面這些相關記錄,請直接查看。",
            Self::En => "The model could not finish summarizing, but these related records were found — please review them directly.",
            Self::Ja => "モデルは要約を完了できませんでしたが、以下の関連記録が見つかりました。直接ご確認ください。",
            Self::Pt => "O modelo não conseguiu concluir o resumo, mas estes registros relacionados foram encontrados — veja-os diretamente.",
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
        }
    }

    pub fn timeline_header_all(self, total: i64) -> String {
        match self {
            Self::ZhHans => format!("该时段共 {total} 条活动记录,全部列出:"),
            Self::ZhHant => format!("該時段共 {total} 條活動記錄,全部列出:"),
            Self::En => format!("{total} activity records in this period, all listed:"),
            Self::Ja => format!("この期間の活動記録は計 {total} 件。すべて列挙します:"),
            Self::Pt => format!("{total} registros de atividade neste período, todos listados:"),
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
        };
        if pending > 0 {
            match self {
                Self::ZhHans => format!("{base},另有 {pending} 帧截图待识别。"),
                Self::ZhHant => format!("{base},另有 {pending} 幀截圖待識別。"),
                Self::En => format!("{base}, with {pending} frame(s) still awaiting recognition."),
                Self::Ja => format!("{base}。ほかに認識待ちのフレームが {pending} 件あります。"),
                Self::Pt => format!("{base}, com {pending} quadro(s) aguardando reconhecimento."),
            }
        } else {
            match self {
                Self::ZhHans | Self::ZhHant | Self::Ja => format!("{base}。"),
                Self::En | Self::Pt => format!("{base}."),
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
            }
        } else {
            match self {
                Self::ZhHans => format!("共 {total} 条命中:"),
                Self::ZhHant => format!("共 {total} 條命中:"),
                Self::En => format!("{total} hits:"),
                Self::Ja => format!("計 {total} 件ヒット:"),
                Self::Pt => format!("{total} resultados:"),
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
        }
    }

    pub fn no_match(self, from: &str, to: &str) -> String {
        match self {
            Self::ZhHans => format!("{from} ~ {to} 无匹配记录"),
            Self::ZhHant => format!("{from} ~ {to} 無匹配記錄"),
            Self::En => format!("{from} ~ {to}: no matching records"),
            Self::Ja => format!("{from} ~ {to} 該当する記録はありません"),
            Self::Pt => format!("{from} ~ {to}: nenhum registro correspondente"),
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
            }
        } else {
            match self {
                Self::ZhHans => format!("{from} ~ {to} 按时长排序:"),
                Self::ZhHant => format!("{from} ~ {to} 按時長排序:"),
                Self::En => format!("{from} ~ {to}, sorted by duration:"),
                Self::Ja => format!("{from} ~ {to} 合計時間順:"),
                Self::Pt => format!("{from} ~ {to}, ordenado por duração:"),
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
        }
    }

    pub fn count_suffix(self, n: usize) -> String {
        match self {
            Self::ZhHans => format!("{n} 次"),
            Self::ZhHant => format!("{n} 次"),
            Self::En => format!("{n} sessions"),
            Self::Ja => format!("{n} 回"),
            Self::Pt => format!("{n} sessões"),
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
        }
    }

    pub fn err_need_range(self, tool: &str) -> String {
        match self {
            Self::ZhHans => format!("{tool} 需要 date_from 和 date_to(YYYY-MM-DD)"),
            Self::ZhHant => format!("{tool} 需要 date_from 和 date_to(YYYY-MM-DD)"),
            Self::En => format!("{tool} requires date_from and date_to (YYYY-MM-DD)"),
            Self::Ja => format!("{tool} には date_from と date_to(YYYY-MM-DD)が必要です"),
            Self::Pt => format!("{tool} requer date_from e date_to (YYYY-MM-DD)"),
        }
    }

    pub fn err_bad_date(self, field: &str, val: &str) -> String {
        match self {
            Self::ZhHans => format!("{field} 不是有效日期: {val}"),
            Self::ZhHant => format!("{field} 不是有效日期: {val}"),
            Self::En => format!("{field} is not a valid date: {val}"),
            Self::Ja => format!("{field} は有効な日付ではありません: {val}"),
            Self::Pt => format!("{field} não é uma data válida: {val}"),
        }
    }

    pub fn err_from_after_to(self) -> &'static str {
        match self {
            Self::ZhHans => "date_from 晚于 date_to",
            Self::ZhHant => "date_from 晚於 date_to",
            Self::En => "date_from is later than date_to",
            Self::Ja => "date_from が date_to より後になっています",
            Self::Pt => "date_from é posterior a date_to",
        }
    }

    pub fn err_range_too_long(self) -> &'static str {
        match self {
            Self::ZhHans => "时间跨度超过 366 天,请缩小范围",
            Self::ZhHant => "時間跨度超過 366 天,請縮小範圍",
            Self::En => "The range exceeds 366 days; please narrow it",
            Self::Ja => "期間が 366 日を超えています。範囲を狭めてください",
            Self::Pt => "O intervalo excede 366 dias; reduza-o",
        }
    }

    pub fn err_from_in_future(self) -> &'static str {
        match self {
            Self::ZhHans => "date_from 在未来",
            Self::ZhHant => "date_from 在未來",
            Self::En => "date_from is in the future",
            Self::Ja => "date_from が未来の日付です",
            Self::Pt => "date_from está no futuro",
        }
    }

    pub fn err_keywords_empty(self) -> &'static str {
        match self {
            Self::ZhHans => "keywords 不能为空",
            Self::ZhHant => "keywords 不能為空",
            Self::En => "keywords must not be empty",
            Self::Ja => "keywords を空にはできません",
            Self::Pt => "keywords não pode estar vazio",
        }
    }

    pub fn err_item_too_long(self, field: &str) -> String {
        match self {
            Self::ZhHans => format!("{field} 里有超过 64 字符的项"),
            Self::ZhHant => format!("{field} 裡有超過 64 字元的項"),
            Self::En => format!("{field} contains an item longer than 64 characters"),
            Self::Ja => format!("{field} に 64 文字を超える項目があります"),
            Self::Pt => format!("{field} contém um item com mais de 64 caracteres"),
        }
    }

    pub fn err_title_kw_too_long(self) -> &'static str {
        match self {
            Self::ZhHans => "title_keyword 过长(≤64 字符)",
            Self::ZhHant => "title_keyword 過長(≤64 字元)",
            Self::En => "title_keyword is too long (max 64 characters)",
            Self::Ja => "title_keyword が長すぎます(64 文字以内)",
            Self::Pt => "title_keyword é longo demais (máx. 64 caracteres)",
        }
    }

    /// 「问题自立化」改写器的系统提示词(多轮第二问起,先把新问题改写成
    /// 不依赖上下文的自足问题,再让回答器以零历史状态作答——历史污染物理隔离)。
    pub fn rewrite_prompt(self, today: NaiveDate) -> String {
        match self {
            Self::ZhHans => format!(
                "你是问题改写器。根据对话记录,把用户的新问题改写成不依赖上下文也能独立理解的自足问题。\n                 规则:\n                 1. 只做指代消解与信息补全:把\"那个应用/它/这些\"等替换为对话中对应的具体名称;                 相对时间词(昨天/上周)可保留,但连环相对(\"再往前一周呢\")必须换算清楚,今天是 {today}。\n                 2. 不回答问题,不添加对话中不存在的信息,不改变提问意图。\n                 3. 保持新问题原本的语言。\n                 4. 若新问题本已自足,原样输出。\n                 只输出最终问题本身,不要任何解释、前缀或引号。"
            ),
            Self::ZhHant => format!(
                "你是問題改寫器。根據對話記錄,把使用者的新問題改寫成不依賴上下文也能獨立理解的自足問題。\n                 規則:\n                 1. 只做指代消解與資訊補全:把「那個應用程式/它/這些」等替換為對話中對應的具體名稱;                 相對時間詞(昨天/上週)可保留,但連環相對(「再往前一週呢」)必須換算清楚,今天是 {today}。\n                 2. 不回答問題,不添加對話中不存在的資訊,不改變提問意圖。\n                 3. 保持新問題原本的語言。\n                 4. 若新問題本已自足,原樣輸出。\n                 只輸出最終問題本身,不要任何解釋、前綴或引號。"
            ),
            Self::En => format!(
                "You are a question rewriter. Using the conversation, rewrite the user's new                  question into a self-contained question that is fully understandable without                  context.\nRules:\n                 1. Only resolve references and fill in missing specifics: replace \"that app /                  it / those\" with the concrete names from the conversation; relative time words                  (yesterday / last week) may stay, but chained relatives (\"and the week before                  that?\") must be resolved — today is {today}.\n                 2. Do not answer the question, do not add information absent from the                  conversation, do not change the intent.\n                 3. Keep the question's original language.\n                 4. If the question is already self-contained, output it unchanged.\n                 Output only the final question — no explanation, prefix or quotes."
            ),
            Self::Ja => format!(
                "あなたは質問リライターです。会話の記録をもとに、ユーザーの新しい質問を、                 文脈なしでも単独で理解できる自足した質問に書き換えてください。\nルール:\n                 1. 指示語の解決と情報の補完のみ行う:「あのアプリ/それ/これら」などを会話中の                 具体的な名前に置き換える。相対的な時間語(昨日/先週)は残してよいが、                 連鎖した相対表現(「そのさらに前の週は?」)は必ず換算する。今日は {today}。\n                 2. 質問に回答しない。会話にない情報を追加しない。意図を変えない。\n                 3. 質問の元の言語を保つ。\n                 4. すでに自足している場合はそのまま出力する。\n                 最終的な質問だけを出力し、説明・前置き・引用符は付けないこと。"
            ),
            Self::Pt => format!(
                "Você é um reescritor de perguntas. Com base na conversa, reescreva a nova                  pergunta do usuário como uma pergunta autossuficiente, compreensível sem                  contexto.\nRegras:\n                 1. Apenas resolva referências e complete informações: substitua \"aquele app /                  ele / esses\" pelos nomes concretos da conversa; palavras de tempo relativo                  (ontem / semana passada) podem ficar, mas relativos encadeados (\"e na semana                  anterior?\") devem ser resolvidos — hoje é {today}.\n                 2. Não responda à pergunta, não adicione informações ausentes da conversa, não                  mude a intenção.\n                 3. Mantenha o idioma original da pergunta.\n                 4. Se a pergunta já for autossuficiente, devolva-a inalterada.\n                 Devolva apenas a pergunta final — sem explicações, prefixos ou aspas."
            ),
        }
    }

    /// 同会话并发拒的用户可见文案(唯一直接展示给用户而非模型的条目)。
    pub fn err_conversation_busy(self) -> &'static str {
        match self {
            Self::ZhHans => "这个会话正在回答上一个问题,等它完成或点停止后再发。",
            Self::ZhHant => "這個會話正在回答上一個問題,等它完成或按停止後再發送。",
            Self::En => {
                "This conversation is still answering the previous question — wait for it to finish or press Stop."
            }
            Self::Ja => "この会話は前の質問に回答中です。完了を待つか、停止を押してから送信してください。",
            Self::Pt => {
                "Esta conversa ainda está respondendo à pergunta anterior — aguarde ou pressione Parar."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_prompt_embeds_today_in_all_langs() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        for lang in [
            ChatLang::ZhHans,
            ChatLang::ZhHant,
            ChatLang::En,
            ChatLang::Ja,
            ChatLang::Pt,
        ] {
            let p = lang.rewrite_prompt(today);
            assert!(p.contains("2026-07-20"), "{lang:?} 缺 today");
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
        // 旧前端没传 → 维持历史行为(简中);认不出的 → 英文
        assert_eq!(ChatLang::from_tag(None), ChatLang::ZhHans);
        assert_eq!(ChatLang::from_tag(Some("fr")), ChatLang::En);
    }

    #[test]
    fn system_prompt_language_policy_present() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 8).unwrap();
        assert!(ChatLang::En.system_prompt(d).contains("reply in English"));
        assert!(ChatLang::ZhHans.system_prompt(d).contains("简体中文"));
        assert!(ChatLang::Ja.system_prompt(d).contains("日本語"));
    }
}
