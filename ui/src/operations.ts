export type OperationKind =
    | "check-fonts"
    | "check-network"
    | "install-font"
    | "remove-font"
    | "rollback-font"
    | "save-version"
    | "check-app"
    | "install-app"
    | "refresh-catalog";
export type Operation = {
    kind: OperationKind;
    title: string;
    detail: string;
  };
