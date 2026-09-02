# How to import a character for a user using the command from admin.rs (import-character)

1. export ARENA_IMPORT_TOKEN ( eg. export ARENA_IMPORT_TOKEN="smelly-camel" )

2. Send command:
	curl -i -X POST \
  -H "Content-Type: application/json" \
  -H "X-Import-Token: $ARENA_IMPORT_TOKEN" \
  --data-binary @<your-character>.json \
  http://127.0.0.1:8000/api/dev/v1/import-character

3. You should receive HTTP/1.1 200 OK --> it worked

# To export:

\copy (
  SELECT json_build_object(
    'userId', c.user_id,
    'character', c.character,
    'data', c.data,
    'inventory', c.inventory,
    'wallet', c.wallet,
    'town', c.town,
    'server_state', c.server_state,
    'quests', COALESCE(
      (SELECT json_agg(q) FROM quests q WHERE q.character_id = c.id),
      '[]'::json
    )
  )
  FROM characters c
  WHERE c.user_id = '<your-userid>'
) TO 'character_full_export.json';