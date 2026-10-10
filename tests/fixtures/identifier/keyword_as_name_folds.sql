-- A **non-reserved keyword used as a name** folds too. Postgres's scanner downcases
-- the word before classifying it, so `CREATE TABLE Sequence` creates `sequence` there;
-- pglet lexes `Sequence` to a `Keyword` token rather than an `Identifier`, so a fold
-- written only for the identifier arm would leave this one `42P01` while every
-- ordinary-identifier fixture stayed green.
-- @setup
CREATE TABLE Sequence (a INT);
INSERT INTO SEQUENCE VALUES (7);

-- @test
SELECT a FROM sequence;
