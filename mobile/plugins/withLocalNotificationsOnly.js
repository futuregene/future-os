const { withEntitlementsPlist } = require("@expo/config-plugins");

// expo-notifications supports both local and remote notifications, and its
// default iOS plugin unconditionally adds the APNs entitlement. FutureOS only
// schedules local task reminders, so remove that remote-notification capability
// after the upstream plugin has completed its native configuration.
// Register this plugin BEFORE expo-notifications; entitlement mods run in
// reverse registration order.
module.exports = function withLocalNotificationsOnly(config) {
  return withEntitlementsPlist(config, (mod) => {
    delete mod.modResults["aps-environment"];
    return mod;
  });
};
