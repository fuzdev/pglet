-- **Error precedence**, the `SET`-target member — see
-- `returning_from_wildcard_yields_to_missing_source.sql` for the class and its control.
--
-- `SET nosuchcol = 1` names a column the target lacks: `42703` on 18.6, and `42703`
-- here, from `execute_update`'s assignment-index lookup. The `RETURNING *` beside it
-- resolves and expands at plan time over a namespace both relations are in, so it must
-- pass silently and let this error stand; a plan-time refusal over the bare spelling
-- answered `0A000` instead.
-- @setup
CREATE TABLE wps_t (id INT, v TEXT);
CREATE TABLE wps_s (id INT, v TEXT, k INT);
INSERT INTO wps_t VALUES (1, 'TARGET');
INSERT INTO wps_s VALUES (1, 'SOURCE', 7);

-- @test
-- @expect-error: 42703 — the unknown SET target column, on a statement whose RETURNING * resolves
UPDATE wps_t SET nosuchcol = 1 FROM wps_s s WHERE s.id = wps_t.id RETURNING *;
