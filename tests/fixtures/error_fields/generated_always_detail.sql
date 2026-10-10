-- 428C9 carries a detail and, on the INSERT route only, a hint — and no `s`/`t`/`c`,
-- even though the engine holds both names. pglet sends what 18.6 sends, not what it
-- happens to know.
-- @setup
CREATE TABLE t_ga (a INT GENERATED ALWAYS AS IDENTITY, b INT);

-- @test
-- @expect-error: a non-DEFAULT value into a GENERATED ALWAYS column
-- @expect-message
INSERT INTO t_ga (a, b) VALUES (1, 2);
