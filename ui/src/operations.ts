export type OperationKind =
    | "check-fonts"
    | "install-font"
    | "check-app"
    | "install-app"
    | "refresh-catalog";
export type Operation = {
    kind: OperationKind;
    title: string;
    detail: string;
  };
