-- The row the `DELETE` removes holds `q` under the expression unique index until it goes;
-- nulling its child moves the child's key onto `q`, which is no longer held.
-- @setup
CREATE TABLE t (id text PRIMARY KEY, pad text, parent text REFERENCES t ON DELETE SET NULL);
CREATE UNIQUE INDEX t_u ON t (coalesce(parent, pad));
INSERT INTO t VALUES ('a', 'q', NULL);
INSERT INTO t VALUES ('b', 'q', 'a');
DELETE FROM t WHERE id = 'a';
-- @test
-- @profile-lock
SELECT id, pad, parent FROM t ORDER BY id;
