package cn.future_os.shareintent;

import java.io.File;
import java.io.IOException;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/** Bounded staging IO, independent of Android so failure paths can run on the JVM. */
final class ShareFileCopier {
  static final long MAX_FILE_BYTES = 25L * 1024 * 1024;
  static final long MAX_TOTAL_BYTES = 50L * 1024 * 1024;
  static final int MAX_FILES = 10;

  static final class LimitExceededException extends IOException {
    private static final long serialVersionUID = 1L;
    LimitExceededException() { super("Share import limit exceeded"); }
  }

  static final class Budget {
    private long remaining;
    private int files;

    Budget() { this(MAX_TOTAL_BYTES); }
    Budget(long remaining) { this.remaining = remaining; }

    // Count attempted items, including unreadable and oversized providers.
    void beginFile() throws LimitExceededException {
      if (files >= MAX_FILES || remaining <= 0) throw new LimitExceededException();
      files++;
    }
  }

  /** Owns the input stream; removes the target on every unsuccessful exit. */
  static long copy(InputStream input, File target, Budget budget) throws IOException {
    boolean complete = false;
    long total = 0;
    try {
      try (InputStream source = input; OutputStream output = new FileOutputStream(target)) {
        byte[] buffer = new byte[8192];
        for (;;) {
          // Read at most one byte past a boundary to distinguish exact-size EOF
          // from overflow. Never write that extra byte, or buffer the whole file.
          long allowed = Math.min(MAX_FILE_BYTES - total, budget.remaining);
          int count = source.read(buffer, 0, (int) Math.min(buffer.length, allowed + 1));
          if (count < 0) break;
          if (count == 0) throw new IOException("Share source made no progress");
          budget.remaining = Math.max(0, budget.remaining - count);
          if (count > allowed) throw new LimitExceededException();
          output.write(buffer, 0, count);
          total += count;
        }
        if (total == 0) throw new IOException("Empty share source");
      }
      complete = true;
      return total;
    } finally {
      if (!complete && target.exists() && !target.delete()) {
        throw new IOException("Cannot remove incomplete share file");
      }
    }
  }

  private ShareFileCopier() {}
}
