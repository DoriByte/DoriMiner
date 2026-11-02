const minutes = (hhmm) => {
  const [h, m] = hhmm.split(':').map(Number);
  return h * 60 + m;
};

function slotActive(slot, date) {
  const now = date.getHours() * 60 + date.getMinutes();
  const from = minutes(slot.from);
  const to = minutes(slot.to);
  const today = date.getDay();
  if (from < to) return slot.days.includes(today) && now >= from && now < to;
  const yesterday = (today + 6) % 7;
  return (slot.days.includes(today) && now >= from) || (slot.days.includes(yesterday) && now < to);
}

function wantsMining(schedule, date = new Date()) {
  if (!schedule.enabled) return true;
  return schedule.slots.some((s) => slotActive(s, date));
}

function profileNow(config, date = new Date()) {
  if (config.schedule.enabled) {
    const slot = config.schedule.slots.find((s) => s.profileId && slotActive(s, date));
    if (slot && config.profiles.some((p) => p.id === slot.profileId)) return slot.profileId;
  }
  return config.activeProfile;
}

module.exports = { wantsMining, slotActive, profileNow };
