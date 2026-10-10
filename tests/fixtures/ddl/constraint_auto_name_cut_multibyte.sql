-- The cut is in bytes and never splits a character (`pg_mbcliplen`): a quoted mixed-case
-- table name of two-byte characters and column names of three-byte ones, so a byte budget
-- lands inside a character and the name gives that character up whole — a result shorter
-- than 63 bytes. The two columns differ only in their last character, past the cut, so
-- the second's names take the digit. `chars` is what a cut counted in characters would
-- move.
-- @setup
CREATE TABLE cac4p (id INT PRIMARY KEY);
CREATE TABLE "Cac4üüüüüüüüüüüüüüüüüüüüüüüüüüü" (
  "C€€€€€€€€€€€€€y" INT UNIQUE CHECK ("C€€€€€€€€€€€€€y" > 0) REFERENCES cac4p,
  "C€€€€€€€€€€€€€x" INT UNIQUE CHECK ("C€€€€€€€€€€€€€x" > 0) REFERENCES cac4p,
  "Id" INT PRIMARY KEY,
  k INT,
  CHECK (k > "Id"),
  UNIQUE ("C€€€€€€€€€€€€€x", "C€€€€€€€€€€€€€y"));
-- @test
SELECT contype::text AS contype, conname::text AS conname, length(conname::text) AS chars
FROM pg_constraint WHERE conname LIKE 'Cac4%' AND contype <> 'n' ORDER BY contype, conname;
