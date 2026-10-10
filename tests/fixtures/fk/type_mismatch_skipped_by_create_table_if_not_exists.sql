-- `CREATE TABLE IF NOT EXISTS` over an existing table skips the statement — its unimplementable
-- foreign key included — on both engines.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (z INT);
CREATE TABLE IF NOT EXISTS ftc (b TEXT REFERENCES ftp);

-- @test
SELECT conname, contype FROM pg_constraint WHERE contype = 'f' ORDER BY conname;
