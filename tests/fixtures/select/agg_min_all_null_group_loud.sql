-- The aggregate operand gate must be **static**, not data-dependent. Postgres
-- resolves `min(boolean)` at parse time, so it is `42883` whether the group holds
-- a thousand rows, all-NULL rows, or none. Gating on observed values alone let an
-- all-NULL group through — the same SQL over the same schema erroring or not
-- depending on the data, the worst shape for a test-fixture engine (green on an
-- empty seed, red once populated).
-- @setup
CREATE TABLE agg_nullgrp (g INT, b BOOL);
INSERT INTO agg_nullgrp VALUES (1, NULL), (2, NULL);
-- @test
-- @expect-error: no min(boolean)
SELECT g, MIN(b) FROM agg_nullgrp GROUP BY g;
