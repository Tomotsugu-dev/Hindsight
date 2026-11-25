import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Brain, Cloud, HardDrive, ListFilter, Settings2 } from "lucide-react";
import { api, type ReasoningLevel } from "../../api/hindsight";
import { InlinePicker } from "../../components/InlinePicker/InlinePicker";
import { routeName, shortModelName } from "../../components/ReportModel/reportModelOptions";
import { ROUTES } from "../../config/nav";
import { logError } from "../../lib/logger";
import {
  hasLevels,
  normalizeThinking,
  thinkingLabelKey,
  thinkingModes,
} from "../../lib/thinkingModes";
import { AI_PROFILES_HREF, type AiProfilesState } from "../../state/aiProfiles";
import {
  decodeModelValue,
  encodeModelRef,
  findAdvanced,
  patchAdvanced,
} from "../../state/reportModel";
import { useSettings } from "../../state/settings";
import { profileLabel } from "../../utils/profileLabel";
import {
  chatCloudConnection,
  chatLocalModelName,
  chatModel,
  readyCloudConnections,
} from "./chatRouting";

/** The menu value of the default profile, which has no id. */
const MAIN_PROFILE = "";

/**
 * The pickers in the bottom row of the chat input: what data the AI can see,
 * which model answers, and how hard it thinks.
 * A change applies to the next question. Answers already given stay the same.
 */
export default function ComposerPickers({ profiles }: { profiles: AiProfilesState }) {
  return (
    <>
      <ProfilePicker state={profiles} />
      <ModelPicker />
      <ThinkingPicker />
    </>
  );
}

function ProfilePicker({ state }: { state: AiProfilesState }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const options = [
    { value: MAIN_PROFILE, label: t("aiProfiles.main") },
    ...state.list.map(([id, p]) => ({ value: id, label: p.name })),
  ];
  const value = state.selected ?? MAIN_PROFILE;
  const label = options.find((o) => o.value === value)?.label ?? t("aiProfiles.main");

  return (
    <InlinePicker
      value={value}
      options={options}
      onChange={(v) => state.select(v === MAIN_PROFILE ? null : v)}
      label={label}
      icon={ListFilter}
      title={t("aiProfiles.pickerTitle")}
      up
      action={{
        label: t("aiProfiles.manage"),
        icon: Settings2,
        onClick: () => void navigate(AI_PROFILES_HREF),
      }}
    />
  );
}

/**
 * Chat sends tools with every question, so the menu lists only the local models
 * that can call tools. The choice is only `picks.chat`; the reports keep their own.
 */
function ModelPicker() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { settings, reload } = useSettings();
  const [localFiles, setLocalFiles] = useState<string[]>([]);
  const [toollessOnly, setToollessOnly] = useState(false);

  const refreshLocal = useCallback(() => {
    api
      .listLocalModels()
      .then((all) => {
        const mains = all.filter((m) => !m.isMmproj);
        const usable = mains.filter((m) => m.supportsTools !== false);
        setLocalFiles(usable.map((m) => m.filename));
        setToollessOnly(usable.length === 0 && mains.length > 0);
      })
      .catch((e) => logError("chat.listModels", e));
  }, []);

  useEffect(refreshLocal, [refreshLocal]);

  if (!settings) return null;
  const ai = settings.ai;
  const cloud = chatCloudConnection(ai);
  const local = chatLocalModelName(ai);
  const value = cloud
    ? encodeModelRef({ kind: "cloud", connection: cloud.id })
    : local
      ? encodeModelRef({ kind: "local", file: local })
      : null;

  const options = [
    ...readyCloudConnections(ai).map((conn) => ({
      value: encodeModelRef({ kind: "cloud", connection: conn.id }),
      label: profileLabel(conn),
      icon: Cloud,
      cloud: true,
      title: conn.endpoint,
    })),
    ...localFiles.map((file) => ({
      value: encodeModelRef({ kind: "local", file }),
      label: shortModelName(file),
      icon: HardDrive,
      title: file,
    })),
  ];

  const choose = async (next: string) => {
    const model = decodeModelValue(next);
    if (!model || model.kind === "inherit") return;
    try {
      await api.setTaskModel("chat", model);
      await reload();
    } catch (e) {
      logError("chat.setModel", e);
    }
  };

  return (
    <InlinePicker
      value={value}
      options={options}
      onChange={(v) => void choose(v)}
      label={routeName({ cloud, local }, t)}
      icon={cloud ? Cloud : HardDrive}
      cloud={Boolean(cloud)}
      title={t("aiSummary.model.modelGroup")}
      note={
        options.length === 0
          ? t(toollessOnly ? "chat.model.noToolUseModels" : "chat.model.empty")
          : undefined
      }
      onOpen={refreshLocal}
      up
      action={{
        label: t("chat.model.manage"),
        icon: Settings2,
        onClick: () => void navigate(ROUTES.settingsAiModels),
      }}
    />
  );
}

/** Reads and writes `ai.advanced` for chat and the model in use. */
function ThinkingPicker() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  if (!settings) return null;
  const ai = settings.ai;
  const model = chatModel(ai);
  const provider = chatCloudConnection(ai)?.dialect ?? null;
  const modes = thinkingModes(provider, true);
  const stored = model ? (findAdvanced(ai, "chat", model)?.reasoning ?? "auto") : "auto";
  const mode = normalizeThinking(stored, modes);
  const labelOf = (m: string) => t(thinkingLabelKey(m, modes));
  const note = hasLevels(modes)
    ? "chat.thinkingMode.hintLevels"
    : provider === "openai"
      ? "chat.thinkingMode.hintOpenai"
      : "chat.thinkingMode.hint";

  const choose = (next: string) => {
    if (!model || next === stored) return;
    update({
      ai: {
        ...ai,
        advanced: patchAdvanced(ai, "chat", model, {
          reasoning: next === "auto" ? null : (next as ReasoningLevel),
        }),
      },
    });
  };

  return (
    <InlinePicker
      value={mode}
      options={modes.map((m) => ({ value: m, label: labelOf(m) }))}
      onChange={choose}
      label={labelOf(mode)}
      icon={Brain}
      title={t("chat.thinkingMode.label")}
      note={t(note)}
      disabled={!model}
      up
    />
  );
}
