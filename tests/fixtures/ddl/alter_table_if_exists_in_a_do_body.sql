-- The skip is decided where the statement runs, not where it is planned: a `DO` body's
-- statements are all planned before the first runs, so a table the body itself creates
-- is absent at planning time and present when the `ALTER` runs — and the column is added.
-- @setup
DO $$ BEGIN
    CREATE TABLE atie21 (a INT);
    ALTER TABLE IF EXISTS atie21 ADD COLUMN b INT DEFAULT 3;
    INSERT INTO atie21 (a) VALUES (1);
END $$;
-- @test
SELECT * FROM atie21;
