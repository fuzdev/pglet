-- An error the first row raises only after parse analysis — here `1/0` — loses to the
-- second row's length, which 18.6 refuses in parse analysis.
-- @setup
CREATE TABLE vldz (a INT, b INT);

-- @test
-- @expect-error: VALUES lists must all be the same length (42601), ahead of the first row's 22012
-- @expect-message
INSERT INTO vldz VALUES (1 / 0, 2), (3);
