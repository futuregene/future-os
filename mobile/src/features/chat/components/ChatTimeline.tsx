import type { TFunction } from "i18next";
import { History } from "lucide-react-native";
import { ActivityIndicator, FlatList, Platform, Pressable, RefreshControl, Text, View, type FlatListProps } from "react-native";
import type { useRemote } from "../../../remote/RemoteContext";
import type { TimelineItem } from "../../../remote/types";
import { spacing } from "../../../theme/tokens";
import { useTheme } from "../../../theme/ThemeContext";
import { useStyles } from "../chatStyles";
import type { useChatScroll } from "../useChatScroll";
import type { useQuestionNav } from "../useQuestionNav";
import type { useTimelinePaging } from "../useTimelinePaging";
import { COMPOSER_FADE_CLEARANCE } from "../utils";
import { FloatingTimelineButton } from "./FloatingTimelineButton";
import { QuestionNavControl } from "./QuestionNavControl";

export type SyncNoticeState = "syncing" | "retrying" | "waitingNetwork";
const SYNC_NOTICE_KEYS: Record<SyncNoticeState, string> = {
  syncing: "chat.syncingLatest",
  retrying: "chat.syncRetrying",
  waitingNetwork: "chat.syncWaitingNetwork",
};
function TimelineItemGap() {
  const styles = useStyles();
  return <View style={styles.itemGap} />;
}
function TimelineFlexSpacer() { return <View />; }
interface Props {
  remote: ReturnType<typeof useRemote>;
  t: TFunction;
  invertedTranscriptItems: TimelineItem[];
  scroll: ReturnType<typeof useChatScroll>;
  questionNav: ReturnType<typeof useQuestionNav>;
  paging: ReturnType<typeof useTimelinePaging>;
  renderTimelineItem: NonNullable<FlatListProps<TimelineItem>["renderItem"]>;
  onChatScroll: FlatListProps<TimelineItem>["onScroll"];
  loadingHistory: boolean;
  pullRefresh: () => void;
  pullRefreshActive: boolean;
  showSyncNotice: boolean;
  noticeState: SyncNoticeState;
}
export function ChatTimeline({ remote, t, invertedTranscriptItems, scroll, questionNav, paging,
  renderTimelineItem, onChatScroll, loadingHistory, pullRefresh, pullRefreshActive, showSyncNotice, noticeState }: Props) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { listRef } = scroll;
  const { showLoadOlderHint, pagingActive, loadOlder, onListLayout, onContentSizeChange,
    onScrollBeginDrag, onScrollEndDrag, onMomentumScrollBegin, onMomentumScrollEnd } = paging;
  return (
    <View style={styles.timelineViewport}>
      {/* Inverted data puts latest at offset zero. Reading-mode native
                  anchoring handles subsequent row layout changes. */}
      <FlatList
        contentContainerStyle={[
          styles.timeline,
          {
            // The scroll container is inverted, so logical top padding is
            // rendered at the visual bottom. The composer itself now
            // participates in flex layout; only its overlaid fade needs
            // clearance inside the list.
            paddingTop: COMPOSER_FADE_CLEARANCE + spacing.lg,
          },
          remote.timeline.items.length === 0 && styles.emptyTimeline,
        ]}
        data={invertedTranscriptItems}
        initialNumToRender={10}
        inverted
        key={remote.selectedSessionId || "draft"}
        keyExtractor={(item) => item.id}
        refreshControl={
          <RefreshControl
            colors={[colors.accent]}
            enabled={!remote.draft && remote.desktopOnline}
            onRefresh={pullRefresh}
            progressViewOffset={spacing.lg}
            refreshing={pullRefreshActive}
            tintColor={colors.accent}
          />
        }
        ListHeaderComponent={
          invertedTranscriptItems.length > 0
            ? TimelineFlexSpacer
            : undefined
        }
        ListHeaderComponentStyle={styles.timelineFlexSpacer}
        ListEmptyComponent={
          remote.timelineError ? (
            <View style={styles.loadingState}>
              <Text style={styles.historyError}>
                {t("chat.historyLoadTimedOut")}
              </Text>
              <Pressable
                accessibilityRole="button"
                onPress={() => void remote.retryTimeline()}
                style={({ pressed }) => [
                  styles.retryButton,
                  pressed && styles.retryPressed,
                ]}
              >
                <Text style={styles.retryLabel}>
                  {t("common.retry")}
                </Text>
              </Pressable>
            </View>
          ) : loadingHistory ? (
            <View style={styles.loadingState}>
              <ActivityIndicator color={colors.accent} />
              <Text style={styles.empty}>
                {t("chat.loadingHistory")}
              </Text>
            </View>
          ) : (
            <Text style={styles.empty}>{t("chat.noHistory")}</Text>
          )
        }
        maintainVisibleContentPosition={
          scroll.maintainVisibleContentPosition
        }
        keyboardDismissMode={
          Platform.OS === "ios" ? "interactive" : "on-drag"
        }
        keyboardShouldPersistTaps="handled"
        // Android selectable Text requests focus/rectangle visibility on
        // long press. Its native auto-scroll ignores our inverted reading
        // anchor and jumps the transcript; selection itself stays enabled.
        scrollsChildToFocus={false}
        automaticallyAdjustContentInsets={false}
        contentInsetAdjustmentBehavior="never"
        automaticallyAdjustKeyboardInsets={false}
        maxToRenderPerBatch={8}
        onContentSizeChange={(width, height) => {
          scroll.onContentSizeChange();
          onContentSizeChange(width, height);
        }}
        onLayout={(event) => {
          scroll.onLayout();
          onListLayout(event.nativeEvent.layout.height);
        }}
        onScrollBeginDrag={(event) => {
          scroll.onScrollBeginDrag();
          questionNav.onScrollBeginDrag();
          onScrollBeginDrag(event);
        }}
        onMomentumScrollBegin={onMomentumScrollBegin}
        onMomentumScrollEnd={onMomentumScrollEnd}
        onScroll={onChatScroll}
        onScrollToIndexFailed={questionNav.onScrollToIndexFailed}
        onScrollEndDrag={onScrollEndDrag}
        ref={listRef}
        renderItem={renderTimelineItem}
        scrollEventThrottle={16}
        scrollIndicatorInsets={{ bottom: 0 }}
        style={styles.timelineList}
        updateCellsBatchingPeriod={32}
        viewabilityConfigCallbackPairs={
          questionNav.viewabilityConfigCallbackPairs
        }
        windowSize={7}
        ItemSeparatorComponent={TimelineItemGap}
      />

      {questionNav.visible && (
        <QuestionNavControl
          hasNext={questionNav.next !== null}
          hasPrevious={questionNav.hasPrevious}
          nextLabel={t("chat.nextPassage")}
          onNext={questionNav.goToNext}
          onPrevious={questionNav.goToPrevious}
          previousLabel={t("chat.previousQuestion")}
          style={styles.questionNav}
        />
      )}

      {(showLoadOlderHint || (remote.olderHistoryFailed && remote.canLoadOlderTimeline)) && (
        <FloatingTimelineButton
          label={t(
            remote.olderHistoryFailed && !showLoadOlderHint
              ? "chat.loadOlderFailed"
              : "chat.loadingOlder",
          )}
          icon={<History color={colors.inkSoft} size={16} />}
          busy={pagingActive || remote.loadingOlderTimeline}
          onPress={() => {
            scroll.onScrollBeginDrag();
            loadOlder();
          }}
          style={styles.loadOlder}
        />
      )}

      {showSyncNotice && (
        <View
          pointerEvents="none"
          style={[
            styles.syncNotice,
            showLoadOlderHint && styles.syncNoticeBelowHistory,
          ]}
          accessibilityLiveRegion="polite"
        >
          {noticeState !== "waitingNetwork" && (
            <ActivityIndicator color={colors.accent} size="small" />
          )}
          <Text style={styles.syncNoticeText}>
            {t(SYNC_NOTICE_KEYS[noticeState])}
          </Text>
        </View>
      )}
    </View>

  );
}
