-- @setup
-- `AT` is UNRESERVED in Postgres (kwlist.h), so a bare `at` is usable as an
-- ordinary column name — in the SELECT list, ORDER BY, and WHERE. The postfix
-- `AT TIME ZONE` operator still parses against that same column (the word reads as
-- a column in nud position, as the operator in led position). pglet must match.
CREATE TABLE ev (id INT, at TIMESTAMPTZ);
INSERT INTO ev VALUES (1, '2024-06-15T12:30:45Z'), (2, '2024-01-02T03:04:05Z');

-- @test
SELECT id, at, to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS iso
FROM ev
WHERE at > '2024-01-01T00:00:00Z'
ORDER BY at ASC;
