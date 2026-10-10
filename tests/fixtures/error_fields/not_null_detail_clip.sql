-- Each `Failing row` field is clipped to 64 **bytes** and then gets `...`; a value of
-- exactly 64 bytes is not ellipsized (`if (vallen <= maxfieldlen)`).
-- @setup
CREATE TABLE t_clip (big TEXT, exact TEXT, req INT NOT NULL);

-- @test
-- @expect-error: the NOT NULL column is omitted
-- @expect-message
INSERT INTO t_clip (big, exact) VALUES (rpad('', 2000, 'x'), rpad('', 64, 'y'));
