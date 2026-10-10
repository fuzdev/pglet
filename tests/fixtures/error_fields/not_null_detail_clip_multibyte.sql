-- The 64-byte clip cuts at a character boundary (`pg_mbcliplen`), so 100 two-byte
-- characters render as 32 of them plus `...`, not 64 of them and not a split sequence.
-- @setup
CREATE TABLE t_clip2 (big TEXT, req INT NOT NULL);

-- @test
-- @expect-error: the NOT NULL column is omitted
-- @expect-message
INSERT INTO t_clip2 (big) VALUES (rpad('', 100, 'é'));
