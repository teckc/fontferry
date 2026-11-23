export type DeliveryPolicy = "autoInstall" | "notifyOnly";

export interface Variant {
  id: string;
  name: string;
  description: string;
  assetPattern: string;
  default: boolean;
}

export interface FontDefinition {
  id: string;
  name: string;
  description: string;
  homepage: string;
  license: {
    name: string;
    url: string;
    spdx: string | null;
    revision: string;
    requiresAcceptance: boolean;
    redistributionAllowed: boolean;
  };
  versionProvider: Record<string, unknown> & { type: string };
  artifactProvider: (Record<string, unknown> & { type: string }) | null;
  deliveryPolicy: DeliveryPolicy;
  versionPolicy: {
    major?: number | null;
    maximumVersion?: string | null;
    updatesThrough?: string | null;
  };
  variants: Variant[];
  platforms: string[];
}

export interface InstalledFont {
  fontId: string;
  version: string;
  variantIds: string[];
  installedAt: string;
  ownedFiles: string[];
  previous: { version: string } | null;
  manualVersion: string | null;
}

export interface UpdateStatus {
  checkedAt?: string | null;
  fromCache?: boolean;
  fontId: string;
  currentVersion: string | null;
  availableVersion: string | null;
  updateAvailable: boolean;
  deliveryPolicy: DeliveryPolicy;
}

export type InstallStage = "check" | "download" | "verify" | "register";

export interface InstallCandidate {
  version: string;
  assetNames: string[];
  identity: string;
}

export interface InstallReport {
  installed: InstalledFont;
  families: string[];
  cleanupPending: string | null;
}

export interface NetworkProbe {
  releasesError: string | null;
  downloadError: string | null;
}

export interface Activity {
  id: string;
  fontId: string | null;
  level: "info" | "warning" | "error";
  message: string;
  createdAt: string;
}

export interface Dashboard {
  updateChannel?: "stable" | "beta";
  githubProxyPort?: number | null;
  scheduleEnabled?: boolean | null;
  fonts: FontDefinition[];
  installed: InstalledFont[];
  statuses: UpdateStatus[];
  activities: Activity[];
}
