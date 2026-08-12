// Shell selector (Phase 1): the React hermes-parity shell runs behind
// ?shell=react or a persisted choice; the vanilla shell stays default so
// secondary views keep working until the migration completes.
const params = new URLSearchParams(location.search);
const chosen = params.get("shell") || localStorage.getItem("ulnclaw.shell") || "vanilla";
if (chosen === "react") {
  void import("./react/main");
} else {
  void import("./main");
}
