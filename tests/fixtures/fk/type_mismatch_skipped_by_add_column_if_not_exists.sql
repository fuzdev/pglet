-- `ADD COLUMN IF NOT EXISTS` over an existing column skips the subcommand, its unimplementable
-- foreign key included.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (b TEXT);
ALTER TABLE ftc ADD COLUMN IF NOT EXISTS b TEXT REFERENCES ftp;

-- @test
SELECT conname, contype FROM pg_constraint WHERE contype = 'f' ORDER BY conname;
