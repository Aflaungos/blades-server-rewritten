DROP TABLE IF EXISTS guild_removals;
DROP TABLE IF EXISTS guild_applications;

ALTER TABLE guilds DROP COLUMN IF EXISTS grandmaster_since;
ALTER TABLE guilds DROP COLUMN IF EXISTS exchange_donation_count;

-- Rank vocabulary back to this codebase's pre-retail stand-ins. ELDER has no
-- pre-retail counterpart; it collapses into the generic MEMBER it was carved out of.
UPDATE guild_members SET rank = 'LEADER'  WHERE rank = 'GRANDMASTER';
UPDATE guild_members SET rank = 'OFFICER' WHERE rank = 'MASTER';
UPDATE guild_members SET rank = 'MEMBER'  WHERE rank = 'ELDER';
