package cn.future_os.shareintent;

import static org.junit.Assert.*;

import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

public class ShareFileCopierTest {
  @Rule public TemporaryFolder temporary = new TemporaryFolder();

  private File target() { return new File(temporary.getRoot(), "staged"); }

  private static class GeneratedInput extends InputStream {
    long remaining;
    long read;
    boolean closed;
    GeneratedInput(long remaining) { this.remaining = remaining; }
    @Override public int read() {
      if (remaining-- <= 0) return -1;
      read++;
      return 1;
    }
    @Override public int read(byte[] bytes, int offset, int length) {
      if (remaining <= 0) return -1;
      int count = (int) Math.min(remaining, length);
      java.util.Arrays.fill(bytes, offset, offset + count, (byte) 1);
      remaining -= count;
      read += count;
      return count;
    }
    @Override public void close() { closed = true; }
  }

  @Test public void exactPerFileLimitIsAcceptedAndSourceClosed() throws Exception {
    GeneratedInput input = new GeneratedInput(ShareFileCopier.MAX_FILE_BYTES);
    File target = target();
    assertEquals(ShareFileCopier.MAX_FILE_BYTES, ShareFileCopier.copy(input, target, new ShareFileCopier.Budget()));
    assertEquals(ShareFileCopier.MAX_FILE_BYTES, target.length());
    assertTrue(input.closed);
  }

  @Test public void endlessProviderIsStoppedAtOneBytePastLimitAndPartialFileRemoved() throws Exception {
    GeneratedInput input = new GeneratedInput(Long.MAX_VALUE);
    File target = target();
    assertThrows(ShareFileCopier.LimitExceededException.class,
      () -> ShareFileCopier.copy(input, target, new ShareFileCopier.Budget()));
    assertEquals(ShareFileCopier.MAX_FILE_BYTES + 1, input.read);
    assertFalse(target.exists());
    assertTrue(input.closed);
  }

  @Test public void totalBudgetIncludesPriorCopies() throws Exception {
    ShareFileCopier.Budget budget = new ShareFileCopier.Budget(5);
    File first = target();
    budget.beginFile();
    ShareFileCopier.copy(new ByteArrayInputStream(new byte[3]), first, budget);
    File second = new File(temporary.getRoot(), "second");
    budget.beginFile();
    GeneratedInput input = new GeneratedInput(100);
    assertThrows(ShareFileCopier.LimitExceededException.class, () -> ShareFileCopier.copy(input, second, budget));
    assertEquals(3, input.read);
    assertFalse(second.exists());
    assertEquals(3, first.length());
    assertThrows(ShareFileCopier.LimitExceededException.class, budget::beginFile);
  }

  @Test public void failedReadsConsumeBudgetAndRemovePartialFile() throws Exception {
    ShareFileCopier.Budget budget = new ShareFileCopier.Budget(3);
    GeneratedInput input = new GeneratedInput(2) {
      @Override public int read(byte[] bytes, int offset, int length) {
        if (read > 0) throw new IllegalStateException("provider failed");
        return super.read(bytes, offset, length);
      }
    };
    File target = target();
    assertThrows(IllegalStateException.class, () -> ShareFileCopier.copy(input, target, budget));
    assertFalse(target.exists());
    assertTrue(input.closed);
    assertThrows(ShareFileCopier.LimitExceededException.class,
      () -> ShareFileCopier.copy(new ByteArrayInputStream(new byte[2]), target, budget));
  }

  @Test public void emptyAndZeroProgressSourcesAreRejected() throws Exception {
    File target = target();
    assertThrows(IOException.class, () -> ShareFileCopier.copy(new ByteArrayInputStream(new byte[0]), target, new ShareFileCopier.Budget()));
    assertFalse(target.exists());
    GeneratedInput stalled = new GeneratedInput(1) {
      @Override public int read(byte[] bytes, int offset, int length) { return 0; }
    };
    assertThrows(IOException.class, () -> ShareFileCopier.copy(stalled, target, new ShareFileCopier.Budget()));
    assertFalse(target.exists());
    assertTrue(stalled.closed);
  }

  @Test public void outputOpenFailureStillClosesInput() throws Exception {
    GeneratedInput input = new GeneratedInput(1);
    File invalid = new File(temporary.getRoot(), "missing/child");
    assertThrows(IOException.class, () -> ShareFileCopier.copy(input, invalid, new ShareFileCopier.Budget()));
    assertTrue(input.closed);
    assertFalse(invalid.exists());
  }

  @Test public void closeFailureDoesNotLeaveASeeminglyCompleteFile() throws Exception {
    InputStream input = new ByteArrayInputStream(new byte[] {1}) {
      @Override public void close() throws IOException { throw new IOException("close failed"); }
    };
    File target = target();
    assertThrows(IOException.class, () -> ShareFileCopier.copy(input, target, new ShareFileCopier.Budget()));
    assertFalse(target.exists());
  }

  @Test public void fileCountBudgetIsBoundedEvenWithoutSuccessfulCopies() throws Exception {
    ShareFileCopier.Budget budget = new ShareFileCopier.Budget();
    for (int i = 0; i < ShareFileCopier.MAX_FILES; i++) budget.beginFile();
    assertThrows(ShareFileCopier.LimitExceededException.class, budget::beginFile);
  }

  @Test public void normalBytesRoundTrip() throws Exception {
    byte[] bytes = new byte[] {0, 1, 2, -1};
    File target = target();
    assertEquals(bytes.length, ShareFileCopier.copy(new ByteArrayInputStream(bytes), target, new ShareFileCopier.Budget()));
    assertArrayEquals(bytes, Files.readAllBytes(target.toPath()));
  }
}
