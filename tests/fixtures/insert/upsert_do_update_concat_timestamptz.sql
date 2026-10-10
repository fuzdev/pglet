-- Same persisted hazard as `update/update_set_concat_timestamptz`, via the upsert
-- `DO UPDATE SET` right-hand side. Two scopes meet there and each must stringify a
-- `timestamptz` with its `+00`: the **existing** row (the target-table resolver,
-- `uictz.a`) and the would-be-inserted row, which reaches the evaluator as the
-- `EXCLUDED` outer frame (`EXCLUDED.a`, both concatenated and cast). The two
-- timestamps differ so the columns also prove each side read the row it names. The
-- `@test` reads the values back — the offset-less string was written **durably**.
-- @profile-lock
-- @setup
CREATE TABLE uictz (id INT PRIMARY KEY, a TIMESTAMPTZ, s TEXT, e TEXT, c TEXT);
INSERT INTO uictz VALUES (1, '2026-07-18 12:30:45+00', NULL, NULL, NULL);
INSERT INTO uictz VALUES (1, '2026-07-20 06:07:08+00', NULL, NULL, NULL)
  ON CONFLICT (id) DO UPDATE
  SET s = 'x=' || uictz.a, e = 'y=' || EXCLUDED.a, c = EXCLUDED.a::text;
-- @test
SELECT id, s, e, c FROM uictz ORDER BY id;
