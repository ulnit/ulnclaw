// Shell selector (Phase 4): the React hermes-parity shell is the default;
// the classic vanilla shell remains available behind ?shell=classic or a
// persisted "vanilla" choice until the migration is retired.
const params = new URLSearchParams(location.search);
const raw = params.get("shell") || localStorage.getItem("ulnclaw.shell") || "react";
const chosen = raw === "classic" ? "vanilla" : raw;
if (chosen === "vanilla") {
  void import("./main");
} else {
  void import("./react/main");
}
