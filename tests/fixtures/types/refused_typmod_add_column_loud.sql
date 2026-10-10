-- `ADD COLUMN` is the other writing position, and the one whose failure would be durable:
-- the column lands in the catalog inside the mutation block, so a refusal that ran late
-- would leave a half-added column every `SELECT *` then describes.
-- @setup
CREATE TABLE rtac (a INT);
-- @expect-error: a character length below 1 at ADD COLUMN
-- @expect-message
-- @test
ALTER TABLE rtac ADD COLUMN c CHAR(0);
