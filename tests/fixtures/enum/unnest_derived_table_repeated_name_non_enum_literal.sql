-- A qualified reference to a FROM-clause `unnest`'s column is typed by the `unnest`, not
-- by another `FROM` relation's column of the same name. `u.m` is the `unnest`'s `text`
-- column and `ue.m` a `mood`. With no relation in scope for the `unnest`, `u.m` took
-- `ue.m`'s descriptor, the derived table's `x.m` exported `mood`'s enum, and `'aaa'` — no
-- member of `mood` — was refused `22P02` where 18.6 compares two strings.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ue (m mood NOT NULL);
INSERT INTO ue VALUES ('happy');

-- @test
SELECT x.m FROM (SELECT u.m FROM ue, unnest(ARRAY['zzz', 'abc']) AS u(m)) x
WHERE x.m > 'aaa' ORDER BY x.m;
