import React from "react";
import { useTranslation } from "react-i18next";
import {
  Cog,
  FlaskConical,
  History,
  Info,
  Sparkles,
  Cpu,
  MessageSquareText,
} from "lucide-react";
import HandyHand from "./icons/HandyHand";
import { useSettings } from "../hooks/useSettings";
import {
  GeneralSettings,
  AdvancedSettings,
  HistorySettings,
  DebugSettings,
  AboutSettings,
  PostProcessingSettings,
  ModelsSettings,
  PhrasesSettings,
} from "./settings";

export type SidebarSection = keyof typeof SECTIONS_CONFIG;

interface IconProps {
  width?: number | string;
  height?: number | string;
  size?: number | string;
  className?: string;
  [key: string]: any;
}

// DotFlow groups the nav by intent — Dictate (produce text), Review (what you produced), System (config) —
// and leads Dictate with Phrases, DotFlow's signature macro/dot-phrase feature, rather than Handy's flat list.
type NavGroup = "dictate" | "review" | "system";

interface SectionConfig {
  labelKey: string;
  icon: React.ComponentType<IconProps>;
  component: React.ComponentType;
  enabled: (settings: any) => boolean;
  group: NavGroup;
}

export const SECTIONS_CONFIG = {
  general: {
    labelKey: "sidebar.general",
    icon: HandyHand,
    component: GeneralSettings,
    enabled: () => true,
    group: "dictate",
  },
  phrases: {
    labelKey: "sidebar.phrases",
    icon: MessageSquareText,
    component: PhrasesSettings,
    enabled: () => true,
    group: "dictate",
  },
  models: {
    labelKey: "sidebar.models",
    icon: Cpu,
    component: ModelsSettings,
    enabled: () => true,
    group: "dictate",
  },
  history: {
    labelKey: "sidebar.history",
    icon: History,
    component: HistorySettings,
    enabled: () => true,
    group: "review",
  },
  postprocessing: {
    labelKey: "sidebar.postProcessing",
    icon: Sparkles,
    component: PostProcessingSettings,
    enabled: (settings) => settings?.post_process_enabled ?? false,
    group: "review",
  },
  advanced: {
    labelKey: "sidebar.advanced",
    icon: Cog,
    component: AdvancedSettings,
    enabled: () => true,
    group: "system",
  },
  debug: {
    labelKey: "sidebar.debug",
    icon: FlaskConical,
    component: DebugSettings,
    enabled: (settings) => settings?.debug_mode ?? false,
    group: "system",
  },
  about: {
    labelKey: "sidebar.about",
    icon: Info,
    component: AboutSettings,
    enabled: () => true,
    group: "system",
  },
} as const satisfies Record<string, SectionConfig>;

const NAV_GROUP_ORDER: NavGroup[] = ["dictate", "review", "system"];
const NAV_GROUP_LABEL_KEYS: Record<NavGroup, string> = {
  dictate: "sidebar.groups.dictate",
  review: "sidebar.groups.review",
  system: "sidebar.groups.system",
};

interface SidebarProps {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeSection,
  onSectionChange,
}) => {
  const { t } = useTranslation();
  const { settings } = useSettings();

  const availableSections = Object.entries(SECTIONS_CONFIG)
    .filter(([_, config]) => config.enabled(settings))
    .map(([id, config]) => ({ id: id as SidebarSection, ...config }));

  const renderNavItem = (section: (typeof availableSections)[number]) => {
    const Icon = section.icon;
    const isActive = activeSection === section.id;
    return (
      <div
        key={section.id}
        className={`relative flex gap-2.5 items-center px-2.5 py-2 w-full rounded-lg cursor-pointer transition-colors text-[13.5px] ${
          isActive
            ? "bg-accent-tint text-text font-medium before:absolute before:left-0.5 before:top-2 before:bottom-2 before:w-[2.5px] before:rounded-full before:bg-accent before:content-['']"
            : "text-muted hover:bg-text/5 hover:text-text"
        }`}
        onClick={() => onSectionChange(section.id)}
      >
        <Icon
          width={17}
          height={17}
          className={`shrink-0 ${isActive ? "text-accent" : "opacity-85"}`}
        />
        <p className="truncate" title={t(section.labelKey)}>
          {t(section.labelKey)}
        </p>
      </div>
    );
  };

  return (
    <div className="flex flex-col w-48 h-full border-e border-hairline px-2.5 py-3 gap-0.5 overflow-y-auto">
      {NAV_GROUP_ORDER.map((group, groupIndex) => {
        const sections = availableSections.filter((s) => s.group === group);
        if (sections.length === 0) return null;
        return (
          <div key={group} className={groupIndex > 0 ? "mt-3" : undefined}>
            <div className="px-2.5 pb-1.5 pt-0.5 text-[10.5px] font-semibold uppercase tracking-[0.09em] text-faint">
              {t(NAV_GROUP_LABEL_KEYS[group])}
            </div>
            <div className="flex flex-col gap-0.5">
              {sections.map(renderNavItem)}
            </div>
          </div>
        );
      })}
    </div>
  );
};
