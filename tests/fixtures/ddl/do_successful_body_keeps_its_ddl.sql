-- The accept side of the failed-`DO` fixtures: a body that **raises nothing** keeps
-- every object it made and every table it dropped stays dropped, with no `EXCEPTION`
-- clause to route it — so an undo that ran on success, or on every block, reddens here.
-- @setup
CREATE TABLE dos_gone (i INT);
INSERT INTO dos_gone VALUES (7);
DO $$ BEGIN
    CREATE TABLE dos_t (a INT);
    INSERT INTO dos_t VALUES (1), (2);
    CREATE TYPE dos_e AS ENUM ('x');
    DROP TABLE dos_gone;
END $$;

-- @test
SELECT (SELECT count(*) FROM dos_t) AS rows_made,
       (SELECT count(*) FROM pg_type WHERE typname = 'dos_e') AS type_made,
       (SELECT count(*) FROM pg_class WHERE relname = 'dos_gone') AS gone_left;
