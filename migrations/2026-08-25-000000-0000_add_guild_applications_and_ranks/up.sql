-- Guild applications, removals/bans, and the retail rank vocabulary.
--
-- WHY: the pre-existing guild subsystem invented its own rank strings ('LEADER',
-- 'OFFICER') and had no notion of a join *request*. Both are wrong against retail:
--
--   * il2cpp `enum GuildRank` (dump.cs:540696) is GRANDMASTER=0, MASTER=1, ELDER=2,
--     MEMBER=3, and captured `GET /guilds/current` bodies carry `"rank":"GRANDMASTER"`
--     / `"rank":"MEMBER"` verbatim (1422 member records in the 20260607 prod snapshot).
--   * il2cpp ships ApplyToGuildRequest / GetGuildApplicationsRequest /
--     ApproveGuildApplicationRequest / DenyGuildApplicationRequest /
--     BanUserFromGuildRequest (dump.cs:462204-462477) — none of which had a server
--     route here at all.
--
-- Idempotent, so it is safe to apply by hand on the existing prod DB as well as
-- through the compose `arena-migrate` ledger.

-- ---------------------------------------------------------------------------
-- 1. Rank vocabulary: invented -> retail.
-- ---------------------------------------------------------------------------
-- 'LEADER' was this codebase's stand-in for the guild founder; retail calls that
-- GRANDMASTER (and reports `grandmasterSinceSecs` alongside it). 'OFFICER' had no
-- retail counterpart at all; MASTER is the rank immediately below GRANDMASTER and
-- is the closest match, so existing OFFICERs are carried over as MASTERs.
UPDATE guild_members SET rank = 'GRANDMASTER' WHERE rank = 'LEADER';
UPDATE guild_members SET rank = 'MASTER'      WHERE rank = 'OFFICER';

-- ---------------------------------------------------------------------------
-- 2. Guild columns the captured wire carries but this schema lacked.
-- ---------------------------------------------------------------------------
-- `guildExchangeDonationCount` and `grandmasterSinceSecs` both appear in every
-- captured guild object (see docs/guilds.md § "Captured wire contract").
ALTER TABLE guilds ADD COLUMN IF NOT EXISTS exchange_donation_count BIGINT NOT NULL DEFAULT 0;
ALTER TABLE guilds ADD COLUMN IF NOT EXISTS grandmaster_since       BIGINT NOT NULL DEFAULT 0;

-- Backfill: the founding grandmaster's join date is when they became grandmaster.
UPDATE guilds g
   SET grandmaster_since = m.join_date
  FROM guild_members m
 WHERE m.guild_id = g.id
   AND m.rank = 'GRANDMASTER'
   AND g.grandmaster_since = 0;

-- ---------------------------------------------------------------------------
-- 3. Join requests ("allow a join").
-- ---------------------------------------------------------------------------
-- One pending application per (guild, user). `state` uses the il2cpp
-- `GuildApplicationState` vocabulary (dump.cs:538821): APPLIED / APPROVED /
-- ACCEPTED / DENIED / REJECTED / INVITED. Only APPLIED rows are pending; approve
-- and deny delete the row (retail's applications list only ever showed pending
-- ones), so the column exists for forward compatibility with INVITED rather than
-- as an audit log.
CREATE TABLE IF NOT EXISTS guild_applications (
    guild_id      TEXT NOT NULL,
    user_id       UUID NOT NULL,
    character_id  UUID NOT NULL,
    state         TEXT NOT NULL DEFAULT 'APPLIED',
    creation_time BIGINT NOT NULL,
    PRIMARY KEY (guild_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_guild_applications_guild ON guild_applications(guild_id, creation_time);
-- A user holds at most one outstanding application at a time
-- (`CanJoinGuildResult.AlreadyAppliedToGuild`, dump.cs:540989).
CREATE UNIQUE INDEX IF NOT EXISTS idx_guild_applications_user ON guild_applications(user_id);

-- ---------------------------------------------------------------------------
-- 4. Removals: the kick/leave re-join cooldown, and permanent bans.
-- ---------------------------------------------------------------------------
-- `CanJoinGuildResult.UserRecentlyRemovedFromGuild` (dump.cs:540993) and the UI
-- string "You have been removed from this guild. You may try to join again in {0}."
-- (UI.Guild.Error.JoinTimeout.Body) establish that a removal blocks re-joining THAT
-- guild for a bounded window. `banned = TRUE` is the unbounded case, backing
-- BanUserFromGuildRequest.
--
-- The window LENGTH is NOT recoverable: it lives in the Unity ScriptableObject
-- field GuildData._admissionTimeoutAfterRemovalFromGuildInSeconds, which is not in
-- the il2cpp dump (signatures only) and never crosses the wire. The server's value
-- is an authored stand-in — see GUILD_REJOIN_COOLDOWN_SECS in server/src/guild.rs.
CREATE TABLE IF NOT EXISTS guild_removals (
    guild_id   TEXT NOT NULL,
    user_id    UUID NOT NULL,
    removed_at BIGINT NOT NULL,
    banned     BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (guild_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_guild_removals_user ON guild_removals(user_id);
