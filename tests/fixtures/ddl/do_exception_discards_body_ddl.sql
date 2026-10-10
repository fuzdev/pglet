-- The catalog half of `do_exception_discards_the_body_writes.sql`: a `CREATE
-- TABLE` in a caught body is discarded too, so the table does not exist
-- afterwards. Measured on 18.6.
--
-- It is a separate fixture because the two travel different channels here — rows
-- roll back through the storage backend's savepoint, the catalog through the
-- block's own `Schema` snapshot — so a fixture seeing only one would score green
-- with the other unimplemented.
-- @setup
CREATE TABLE do_ddl_probe (id INT PRIMARY KEY);
DO $$ BEGIN
    CREATE TABLE do_made_here (a INT);
    INSERT INTO do_ddl_probe VALUES (1);
    INSERT INTO do_ddl_probe VALUES (1);
EXCEPTION WHEN unique_violation THEN NULL;
END $$;

-- @test
SELECT count(*) AS do_made_here_exists
FROM information_schema.tables WHERE table_name = 'do_made_here';
