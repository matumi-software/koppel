import { Button, Card, FirstRunShell, NoteList, PageHead, Row } from '@koppel/react-ui';

/**
 * No password was set while setup was open. Koppel takes the first one only for a while after it
 * starts, so the way on is a restart, then this page again.
 */
export default function SetupClosedPage({ machineName, recheck }: { machineName: string; recheck: () => void }) {
  return (
    <FirstRunShell title="Setup has timed out">
      <PageHead
        title="Setup has timed out"
        lede={`Koppel takes the first password for ${machineName} only for a few minutes after it starts, so a stranger cannot claim a machine left waiting. Restart Koppel, then open this page again.`}
      />
      <Card label="How to restart Koppel">
        <h2>How to restart Koppel</h2>
        <NoteList>
          <li>
            In Docker: <code>docker compose restart koppel</code>
          </li>
          <li>
            Linux package: <code>sudo systemctl restart koppel</code>
          </li>
          <li>
            Windows app: right-click the Koppel icon in the notification area and pick <strong>Restart Koppel</strong>
          </li>
        </NoteList>
      </Card>
      <Row justify="end">
        <Button variant="primary" onPress={recheck}>
          Check again
        </Button>
      </Row>
    </FirstRunShell>
  );
}
