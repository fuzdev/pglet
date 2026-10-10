-- A partial index's long `AND` predicate renders flat in `pg_indexes.indexdef`, as
-- 18.6 prints its n-ary `BoolExpr` — past 64 operands pglet's list is balanced
-- (`Expr::bool_list`), and a row the predicate admits is indexed.
-- @setup
CREATE TABLE il (a INT);
CREATE INDEX il_part ON il (a) WHERE a <> 0 AND a <> 1 AND a <> 2 AND a <> 3 AND a <> 4 AND a <> 5 AND a <> 6 AND a <> 7 AND a <> 8 AND a <> 9 AND a <> 10 AND a <> 11 AND a <> 12 AND a <> 13 AND a <> 14 AND a <> 15 AND a <> 16 AND a <> 17 AND a <> 18 AND a <> 19 AND a <> 20 AND a <> 21 AND a <> 22 AND a <> 23 AND a <> 24 AND a <> 25 AND a <> 26 AND a <> 27 AND a <> 28 AND a <> 29 AND a <> 30 AND a <> 31 AND a <> 32 AND a <> 33 AND a <> 34 AND a <> 35 AND a <> 36 AND a <> 37 AND a <> 38 AND a <> 39 AND a <> 40 AND a <> 41 AND a <> 42 AND a <> 43 AND a <> 44 AND a <> 45 AND a <> 46 AND a <> 47 AND a <> 48 AND a <> 49 AND a <> 50 AND a <> 51 AND a <> 52 AND a <> 53 AND a <> 54 AND a <> 55 AND a <> 56 AND a <> 57 AND a <> 58 AND a <> 59 AND a <> 60 AND a <> 61 AND a <> 62 AND a <> 63 AND a <> 64 AND a <> 65 AND a <> 66 AND a <> 67 AND a <> 68 AND a <> 69;
INSERT INTO il VALUES (100), (5);
-- @no-declared-types: 18.6 sends `name` (19) for `indexname`, where pglet sends
--   `text` (25) — pre-existing catalog-type modelling, not this fixture's subject.
-- @test
SELECT indexname, indexdef, (SELECT count(*) FROM il WHERE a = 100 AND a <> 0 AND a <> 1 AND a <> 2 AND a <> 3 AND a <> 4 AND a <> 5 AND a <> 6 AND a <> 7 AND a <> 8 AND a <> 9 AND a <> 10 AND a <> 11 AND a <> 12 AND a <> 13 AND a <> 14 AND a <> 15 AND a <> 16 AND a <> 17 AND a <> 18 AND a <> 19 AND a <> 20 AND a <> 21 AND a <> 22 AND a <> 23 AND a <> 24 AND a <> 25 AND a <> 26 AND a <> 27 AND a <> 28 AND a <> 29 AND a <> 30 AND a <> 31 AND a <> 32 AND a <> 33 AND a <> 34 AND a <> 35 AND a <> 36 AND a <> 37 AND a <> 38 AND a <> 39 AND a <> 40 AND a <> 41 AND a <> 42 AND a <> 43 AND a <> 44 AND a <> 45 AND a <> 46 AND a <> 47 AND a <> 48 AND a <> 49 AND a <> 50 AND a <> 51 AND a <> 52 AND a <> 53 AND a <> 54 AND a <> 55 AND a <> 56 AND a <> 57 AND a <> 58 AND a <> 59 AND a <> 60 AND a <> 61 AND a <> 62 AND a <> 63 AND a <> 64 AND a <> 65 AND a <> 66 AND a <> 67 AND a <> 68 AND a <> 69) AS hits
FROM pg_indexes WHERE indexname = 'il_part';
