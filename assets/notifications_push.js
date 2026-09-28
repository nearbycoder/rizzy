(() => {
  const panel = document.querySelector("[data-push-settings]");
  if (!panel) return;
  const button = panel.querySelector("button");
  const state = panel.querySelector("[data-push-state]");
  const error = panel.querySelector("[data-push-error]");
  if (!("serviceWorker" in navigator) || !("Notification" in window) || !("PushManager" in window)) {
    button.hidden = true;
    error.textContent = "Push notifications are unavailable in this browser.";
    return;
  }
  const keyBytes = () => {
    const value = panel.dataset.vapidKey;
    const decoded = atob((value + "=".repeat((4 - value.length % 4) % 4)).replace(/-/g, "+").replace(/_/g, "/"));
    return Uint8Array.from(decoded, character => character.charCodeAt(0));
  };
  const refresh = async () => {
    const registration = await navigator.serviceWorker.getRegistration("/");
    const subscription = await registration?.pushManager?.getSubscription();
    const on = Notification.permission === "granted" && !!subscription;
    state.textContent = on ? "ON" : "OFF";
    panel.classList.toggle("notifications--on", on);
    button.hidden = on;
  };
  button.addEventListener("click", async () => {
    button.disabled = true;
    error.textContent = "";
    try {
      if (Notification.permission === "denied") throw new Error("Allow notifications in your browser settings first.");
      const registration = await navigator.serviceWorker.getRegistration("/") ||
        await navigator.serviceWorker.register("/service-worker.js", { scope: "/" });
      if (Notification.permission === "default" && await Notification.requestPermission() !== "granted") {
        throw new Error("Notification permission was not granted.");
      }
      const ready = await navigator.serviceWorker.ready;
      const subscription = await ready.pushManager.getSubscription() ||
        await ready.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes() });
      const { endpoint, keys: { p256dh, auth } } = subscription.toJSON();
      const response = await fetch(panel.dataset.subscriptionsUrl, {
        method: "POST", credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ push_subscription: { endpoint, p256dh_key: p256dh, auth_key: auth } })
      });
      if (!response.ok) {
        await subscription.unsubscribe();
        throw new Error("Could not save this device's subscription.");
      }
      await refresh();
    } catch (cause) {
      error.textContent = cause.message || "Could not turn on push notifications.";
    } finally {
      button.disabled = false;
    }
  });
  refresh().catch(() => {});
})();
