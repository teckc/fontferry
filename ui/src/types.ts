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
  fontId: string;
  currentVersion: string | null;
  availableVersion: string | null;
  updateAvailable: boolean;
  deliveryPolicy: DeliveryPolicy;
}

export interface Activity {
  id: string;
  fontId: string | null;
  level: "info" | "warning" | "error";
  message: string;
  createdAt: string;
}

export interface Dashboard {
  fonts: FontDefinition[];
  installed: InstalledFont[];
  statuses: UpdateStatus[];
  activities: Activity[];
}
