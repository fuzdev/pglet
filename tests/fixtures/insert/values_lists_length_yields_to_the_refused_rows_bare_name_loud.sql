-- The refused row's own cells are transformed before its length is compared, so a column
-- reference in the short row itself is `42703`, not `42601`.
-- @setup
CREATE TABLE vlrn (a INT, b INT);

-- @test
-- @expect-error: column "nosuch" does not exist (42703), ahead of its own row's 42601
-- @expect-message
INSERT INTO vlrn VALUES (1, 2), (nosuch);
