-- A derived table over a window query exports the column the window query's qualifier
-- names, enum identity included. `b.m` is `wtt`'s `text` column; the planner's window
-- arm took the first input column named `m` — `wte`'s, a `mood` — so the derived
-- table's `x.m` carried the enum, and a literal that is no member of `mood` was refused
-- `22P02` where 18.6 compares two strings.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE wte (m mood NOT NULL);
CREATE TABLE wtt (m TEXT NOT NULL);
INSERT INTO wte VALUES ('happy');
INSERT INTO wtt VALUES ('zzzz'), ('abc');

-- @test
SELECT x.m FROM (SELECT b.m, row_number() OVER () AS rn FROM wte a, wtt b) x
WHERE x.m > 'zzz' ORDER BY x.m;
