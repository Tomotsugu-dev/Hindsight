//! 聊天自己的多语言文字：系统提示词、循环里回填给模型的话、降级文案、问题改写器。
//! `ChatLang` 本身和工具的文字在 [`crate::ai::tools`]。
//!
//! 回答语言策略(产品决定):**跟随提问语言优先,界面语言兜底**——
//! 规则写进各语言的系统提示词第 6 条。

use chrono::NaiveDate;

use crate::ai::tools::ChatLang;

impl ChatLang {
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
            Self::Es => format!(
                "Eres el asistente de memoria de pantalla del usuario: los registros de \
                 actividad y el texto que aparece en la pantalla de su ordenador están \
                 indexados, y respondes consultándolos con herramientas. Hoy es {today} \
                 ({wd}).\nReglas:\n\
                 1. Convierte los tiempos relativos (la semana pasada / ayer / el mes pasado) \
                 en fechas concretas antes de consultar.\n\
                 2. Llama a una herramienta cada vez; si una búsqueda no encuentra nada, \
                 reinténtalo con otras palabras clave (sinónimos / otro idioma / términos más \
                 cortos).\n\
                 3. Responde solo a partir de los resultados de las herramientas; si algo no \
                 está en los resultados, di que no lo has encontrado — nunca inventes. Los \
                 totales y la cobertura indicados en la cabecera de un resultado describen el \
                 conjunto completo; los elementos listados son solo una muestra — nunca \
                 afirmes «eso era todo» basándote en la muestra.\n\
                 4. Cita las fuentes con índices entre corchetes al final de la frase, p. ej. \
                 [3]; usa solo índices que aparezcan en los resultados, y cada índice debe \
                 respaldar realmente esa frase (no tomes prestadas citas de búsqueda para \
                 números que vienen de query_stats).\n\
                 5. Se permite Markdown sencillo (negrita / listas / tablas); sin títulos.\n\
                 6. Idioma: responde en el idioma de la pregunta del usuario; si no está \
                 claro, responde en español.\n\
                 7. Sé conciso; expresa las duraciones en horas y minutos; menciona las fechas \
                 para que el usuario pueda verificarlas.\n\
                 8. La línea «Cobertura» al principio de cada resultado rige tu redacción: \
                 solo cuando todos los días activos del intervalo tienen índice de texto de \
                 pantalla y no queda ningún fotograma por reconocer puedes afirmar que algo \
                 «nunca apareció en pantalla»; con cobertura parcial, di que «no se encontró \
                 en la parte indexada», y puedes sugerir activar las capturas y el \
                 reconocimiento de texto de pantalla (o esperar a que termine el \
                 reconocimiento)."
            ),
        }
    }

    // ── engine 循环内回填给模型的文案 ─────────────────────

    /// 历史使用守则:只在"改写失败、消毒历史真的进上下文"的兜底分支追加进
    /// system(engine.rs 的 None 臂)。不放 system_prompt 固定规则——主路径
    /// (首问/改写成功)零历史,放那儿是死条款,每问最多 7 次往返白付 token,
    /// 还让模型对不存在的"历史对话"做无谓对齐。
    /// 只禁"数字/结论":引用编号已有双层硬防御(sanitize_history_content 进入
    /// 侧物理剥除 + bind_citations 输出侧过滤),提示词无需重复管。
    pub fn history_replay_note(self) -> &'static str {
        match self {
            Self::ZhHans => "补充规则:下面的历史对话仅用于理解当前问题的指代(如\"上个月呢\");回答必须基于本轮工具返回的资料,不得沿用历史回答中的数字或结论。",
            Self::ZhHant => "補充規則:下面的歷史對話僅用於理解當前問題的指代(如「上個月呢」);回答必須基於本輪工具回傳的資料,不得沿用歷史回答中的數字或結論。",
            Self::En => "Additional rule: the conversation history below exists only to resolve references in the current question (e.g. \"what about last month?\"); base your answer on tool results fetched this turn, and do not carry over numbers or conclusions from earlier answers.",
            Self::Ja => "補足ルール:以下の対話履歴は現在の質問の指示語(「先月は?」など)を理解するためだけのものです。回答は今回ツールが返した結果に基づき、過去の回答の数値や結論を流用しないでください。",
            Self::Pt => "Regra adicional: o histórico de conversa abaixo serve apenas para resolver referências da pergunta atual (ex. \"e no mês passado?\"); baseie a resposta nos resultados de ferramentas desta rodada e não reaproveite números ou conclusões de respostas anteriores.",
            Self::Es => "Regla adicional: el historial de conversación de abajo solo sirve para resolver las referencias de la pregunta actual (p. ej. «¿y el mes pasado?»); basa tu respuesta en los resultados de herramientas obtenidos en esta ronda y no reutilices números ni conclusiones de respuestas anteriores.",
        }
    }

    pub fn dup_call(self) -> &'static str {
        match self {
            Self::ZhHans => "这个查询刚执行过,结果同上。请换参数,或基于本轮已返回的资料作答。",
            Self::ZhHant => "這個查詢剛執行過,結果同上。請換參數,或基於本輪已回傳的資料作答。",
            Self::En => "This exact query was just executed; same result as above. Change the parameters, or answer from the results already returned this turn.",
            Self::Ja => "この検索は直前に実行済みで、結果は上記と同じです。パラメータを変えるか、今回すでに返された結果に基づいて回答してください。",
            Self::Pt => "Esta mesma consulta acabou de ser executada; o resultado é o mesmo acima. Mude os parâmetros ou responda com os resultados já obtidos nesta rodada.",
            Self::Es => "Esta misma consulta se acaba de ejecutar; el resultado es el mismo de arriba. Cambia los parámetros o responde con los resultados ya obtenidos en esta ronda.",
        }
    }

    pub fn args_format_err(self, e: &impl std::fmt::Display) -> String {
        match self {
            Self::ZhHans => format!("参数格式错误: {e}"),
            Self::ZhHant => format!("參數格式錯誤: {e}"),
            Self::En => format!("Malformed arguments: {e}"),
            Self::Ja => format!("引数の形式が不正です: {e}"),
            Self::Pt => format!("Argumentos malformados: {e}"),
            Self::Es => format!("Argumentos con formato incorrecto: {e}"),
        }
    }

    pub fn args_invalid(self, msg: &str) -> String {
        match self {
            Self::ZhHans => format!("参数校验未通过: {msg}"),
            Self::ZhHant => format!("參數校驗未通過: {msg}"),
            Self::En => format!("Argument validation failed: {msg}"),
            Self::Ja => format!("引数の検証に失敗しました: {msg}"),
            Self::Pt => format!("Falha na validação dos argumentos: {msg}"),
            Self::Es => format!("Falló la validación de los argumentos: {msg}"),
        }
    }

    pub fn tool_exec_failed(self) -> &'static str {
        match self {
            Self::ZhHans => "查询执行失败,请换个方式,或直接基于本轮已返回的资料作答。",
            Self::ZhHant => "查詢執行失敗,請換個方式,或直接基於本輪已回傳的資料作答。",
            Self::En => "The query failed to execute. Try a different approach, or answer from the results already returned this turn.",
            Self::Ja => "検索の実行に失敗しました。別の方法を試すか、今回すでに返された結果に基づいて回答してください。",
            Self::Pt => "A consulta falhou. Tente outra abordagem ou responda com os resultados já obtidos nesta rodada.",
            Self::Es => "La consulta no se pudo ejecutar. Prueba otro enfoque o responde con los resultados ya obtenidos en esta ronda.",
        }
    }

    // 三条回填文案统一限定"本轮工具返回的资料"——旧措辞"(以上)已有资料"在
    // 兜底路径(消毒历史在上下文里)会被读成"包括上轮答案",作为更靠后的消息
    // 直接压过历史使用守则,诱导模型复述陈旧结论而不是承认本轮没查到。
    pub fn steps_exhausted(self) -> &'static str {
        match self {
            Self::ZhHans => "查询步数已用完。请立刻基于本轮工具返回的资料作答;资料不足就直接说明没查到什么。",
            Self::ZhHant => "查詢步數已用完。請立刻基於本輪工具回傳的資料作答;資料不足就直接說明沒查到什麼。",
            Self::En => "You are out of query steps. Answer now from the tool results fetched this turn; if they are insufficient, state plainly what could not be found.",
            Self::Ja => "検索ステップを使い切りました。今回ツールが返した結果に基づいて今すぐ回答してください。不足している場合は、何が見つからなかったかを率直に述べてください。",
            Self::Pt => "As etapas de consulta acabaram. Responda agora com os resultados de ferramentas desta rodada; se forem insuficientes, diga claramente o que não foi encontrado.",
            Self::Es => "Se han agotado los pasos de consulta. Responde ahora con los resultados de herramientas de esta ronda; si son insuficientes, di claramente qué no se ha encontrado.",
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
            Self::Es => "Esta vez no se ha podido completar la consulta (problema del modelo o de la red). Prueba a preguntar de forma más concreta: por ejemplo, con un periodo aproximado («la semana pasada», «el 3 de julio por la tarde») o una palabra clave.",
        }
    }

    pub fn degraded_with_evidence(self) -> &'static str {
        match self {
            Self::ZhHans => "模型没能完成归纳,但查到了下面这些相关记录,请直接查看。",
            Self::ZhHant => "模型沒能完成歸納,但查到了下面這些相關記錄,請直接查看。",
            Self::En => "The model could not finish summarizing, but these related records were found — please review them directly.",
            Self::Ja => "モデルは要約を完了できませんでしたが、以下の関連記録が見つかりました。直接ご確認ください。",
            Self::Pt => "O modelo não conseguiu concluir o resumo, mas estes registros relacionados foram encontrados — veja-os diretamente.",
            Self::Es => "El modelo no ha podido terminar el resumen, pero se han encontrado estos registros relacionados: échales un vistazo directamente.",
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
            Self::Es => format!(
                "Eres un reescritor de preguntas. A partir de la conversación, reescribe la \
                 nueva pregunta del usuario como una pregunta autosuficiente, comprensible \
                 sin contexto.\nReglas:\n\
                 1. Resuelve solo las referencias y completa lo que falte: sustituye «esa app / \
                 eso / esos» por los nombres concretos de la conversación; las palabras de \
                 tiempo relativo (ayer / la semana pasada) pueden quedarse, pero los relativos \
                 encadenados («¿y la semana anterior a esa?») hay que resolverlos — hoy es \
                 {today}.\n\
                 2. No respondas la pregunta, no añadas información que no esté en la \
                 conversación, no cambies la intención.\n\
                 3. Mantén el idioma original de la pregunta.\n\
                 4. Si la pregunta ya es autosuficiente, devuélvela sin cambios.\n\
                 Devuelve solo la pregunta final: sin explicaciones, prefijos ni comillas."
            ),
        }
    }

    /// 同会话并发拒的用户可见文案(唯一直接展示给用户而非模型的条目)。
    pub fn err_nothing_to_regenerate(self) -> &'static str {
        match self {
            Self::ZhHans => "这个会话还没有可重新回答的提问。",
            Self::ZhHant => "這個會話還沒有可重新回答的提問。",
            Self::En => "This conversation has no question to answer again.",
            Self::Ja => "この会話にはやり直せる質問がまだありません。",
            Self::Pt => "Esta conversa ainda não tem uma pergunta para responder novamente.",
            Self::Es => "Esta conversación aún no tiene ninguna pregunta que volver a responder.",
        }
    }

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
            Self::Es => {
                "Esta conversación todavía está respondiendo la pregunta anterior: espera a que termine o pulsa Detener."
            }
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
        let today = NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        for lang in LANGS {
            // 静态文案:非空即可
            for s in [
                lang.dup_call(),
                lang.tool_exec_failed(),
                lang.steps_exhausted(),
                lang.degraded_no_evidence(),
                lang.degraded_with_evidence(),
                lang.err_conversation_busy(),
            ] {
                assert!(!s.is_empty(), "{lang:?} 有空文案");
            }
            // 插值类:参数必须真的出现在产物里
            assert!(lang.system_prompt(today).contains("2026-07-26"), "{lang:?}");
            // 规则数对齐:恒 8 条固定规则;历史使用守则不进 system_prompt
            // (它在 history_replay_note,由 engine 仅在兜底分支追加)。
            // 任何语言臂漏改/多改规则条目,这里立刻红。
            let sp = lang.system_prompt(today);
            assert!(
                sp.contains("\n8. ") && !sp.contains("\n9. "),
                "{lang:?} system_prompt 应恰含 1-8 条规则"
            );
            assert!(!lang.history_replay_note().is_empty(), "{lang:?}");
            assert!(
                lang.rewrite_prompt(today).contains("2026-07-26"),
                "{lang:?}"
            );
            assert!(lang.args_format_err(&"boom").contains("boom"));
            assert!(lang.args_invalid("bad-arg").contains("bad-arg"));
        }
    }

    #[test]
    fn rewrite_prompt_embeds_today_in_all_langs() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        for lang in [
            ChatLang::ZhHans,
            ChatLang::ZhHant,
            ChatLang::En,
            ChatLang::Ja,
            ChatLang::Pt,
            ChatLang::Es,
        ] {
            let p = lang.rewrite_prompt(today);
            assert!(p.contains("2026-07-20"), "{lang:?} 缺 today");
        }
    }

    #[test]
    fn system_prompt_language_policy_present() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 8).unwrap();
        assert!(ChatLang::En.system_prompt(d).contains("reply in English"));
        assert!(ChatLang::ZhHans.system_prompt(d).contains("简体中文"));
        assert!(ChatLang::Ja.system_prompt(d).contains("日本語"));
        assert!(ChatLang::Es.system_prompt(d).contains("en español"));
    }

    /// 历史使用守则与三条循环回填文案必须限定"本轮"——旧措辞"(以上)已有资料"
    /// 在兜底路径会被读成"包括上轮答案",诱导复述陈旧结论(code review 实锤)。
    /// 五语言逐一钉死限定词,防止未来措辞回退。
    #[test]
    fn engine_feedback_strings_scope_to_this_turn() {
        let cases: [(ChatLang, &str); 6] = [
            (ChatLang::ZhHans, "本轮"),
            (ChatLang::ZhHant, "本輪"),
            (ChatLang::En, "this turn"),
            (ChatLang::Ja, "今回"),
            // nesta/desta rodada 两种搭配都合法,断言公共词干
            (ChatLang::Pt, "rodada"),
            // en/de esta ronda 同理
            (ChatLang::Es, "ronda"),
        ];
        for (lang, marker) in cases {
            assert!(
                lang.history_replay_note().contains(marker),
                "{lang:?} history_replay_note 缺本轮限定"
            );
            assert!(
                lang.steps_exhausted().contains(marker),
                "{lang:?} steps_exhausted 缺本轮限定"
            );
            assert!(
                lang.dup_call().contains(marker),
                "{lang:?} dup_call 缺本轮限定"
            );
            assert!(
                lang.tool_exec_failed().contains(marker),
                "{lang:?} tool_exec_failed 缺本轮限定"
            );
        }
    }
}
