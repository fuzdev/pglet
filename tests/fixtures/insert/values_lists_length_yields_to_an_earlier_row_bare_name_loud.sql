-- A column reference in a `VALUES` cell has no relation to resolve against, and 18.6
-- refuses it as it transforms the row (`42703`) — so a first-row reference wins over the
-- second row's length.
-- @setup
CREATE TABLE vlen (a INT, b INT);

-- @test
-- @expect-error: column "nosuch" does not exist (42703), ahead of the second row's 42601
-- @expect-message
INSERT INTO vlen VALUES (1, nosuch + 1), (2);
