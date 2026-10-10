-- Composite (multi-column) FOREIGN KEY: an INSERT whose full (a, b) tuple matches
-- no parent (x, y) violates the FK — even when one column (a=1) matches on its own.
-- @profile-lock
-- @setup
CREATE TABLE cfkv_parent (x INT, y INT, PRIMARY KEY (x, y));
CREATE TABLE cfkv_child (a INT, b INT, FOREIGN KEY (a, b) REFERENCES cfkv_parent (x, y));
INSERT INTO cfkv_parent VALUES (1, 2);

-- @test
-- @expect-error: (1, 99) — a matches a parent x but the full tuple is absent (23503)
INSERT INTO cfkv_child VALUES (1, 99);
