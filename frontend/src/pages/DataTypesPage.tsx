import {
  FileSpreadsheet,
  FileJson,
  FileText,
  FileType,
  Presentation,
  Code,
  CheckCircle,
  Sparkles,
  Mail,
  BookOpen,
  Calendar,
  Contact,
} from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

interface FormatCard {
  name: string;
  extensions: string;
  description: string;
  icon: React.ComponentType<{ className?: string }>;
  category: string;
}

const includedFormats: FormatCard[] = [
  // Data (5)
  { name: "CSV", extensions: ".csv", description: "Comma-separated values", icon: FileSpreadsheet, category: "Data" },
  { name: "JSON", extensions: ".json", description: "JavaScript Object Notation", icon: FileJson, category: "Data" },
  { name: "Excel", extensions: ".xlsx / .xls", description: "Microsoft Excel spreadsheets", icon: FileSpreadsheet, category: "Data" },
  { name: "Parquet", extensions: ".parquet", description: "Columnar storage format", icon: FileType, category: "Data" },
  { name: "TSV", extensions: ".tsv", description: "Tab-separated values", icon: FileSpreadsheet, category: "Data" },
  // Documents (7)
  { name: "PDF", extensions: ".pdf", description: "Portable Document Format", icon: FileText, category: "Documents" },
  { name: "Word", extensions: ".doc / .docx", description: "Microsoft Word documents", icon: FileText, category: "Documents" },
  { name: "PowerPoint", extensions: ".ppt / .pptx", description: "Presentation files", icon: Presentation, category: "Documents" },
  { name: "Rich Text", extensions: ".rtf", description: "Rich Text Format", icon: FileText, category: "Documents" },
  { name: "OpenDocument Text", extensions: ".odt", description: "OpenDocument text files", icon: FileText, category: "Documents" },
  { name: "OpenDocument Sheet", extensions: ".ods", description: "OpenDocument spreadsheets", icon: FileSpreadsheet, category: "Documents" },
  { name: "OpenDocument Slides", extensions: ".odp", description: "OpenDocument presentations", icon: Presentation, category: "Documents" },
  // Email (3)
  { name: "Email Message", extensions: ".eml", description: "Email message files", icon: Mail, category: "Email" },
  { name: "Outlook Email", extensions: ".msg", description: "Microsoft Outlook messages", icon: Mail, category: "Email" },
  { name: "Email Mailbox", extensions: ".mbox", description: "Email mailbox archives", icon: Mail, category: "Email" },
  // Publishing (1)
  { name: "ePub", extensions: ".epub", description: "Electronic publication format", icon: BookOpen, category: "Publishing" },
  // Plain Text (3)
  { name: "Text", extensions: ".txt", description: "Plain text files", icon: FileText, category: "Plain Text" },
  { name: "Markdown", extensions: ".md", description: "Markdown documents", icon: FileType, category: "Plain Text" },
  { name: "HTML", extensions: ".html / .htm", description: "Web page documents", icon: Code, category: "Plain Text" },
  // Other (4)
  { name: "XML", extensions: ".xml", description: "Extensible Markup Language", icon: Code, category: "Other" },
  { name: "RSS Feed", extensions: ".rss", description: "RSS syndication feeds", icon: Code, category: "Other" },
  { name: "Calendar", extensions: ".ics", description: "iCalendar event files", icon: Calendar, category: "Other" },
  { name: "vCard", extensions: ".vcf", description: "Contact card files", icon: Contact, category: "Other" },
];

const DataTypesPage = () => {
  return (
    <div className="space-y-6 max-w-4xl pb-20">
      <div>
        <p className="text-muted-foreground">
          Supported upload formats. Processing availability depends on your installation and the file contents.
        </p>
      </div>

      {/* Supported upload formats grouped by category */}
      {["Data", "Documents", "Email", "Publishing", "Plain Text", "Other"].map((category) => {
        const formats = includedFormats.filter((f) => f.category === category);
        if (formats.length === 0) return null;
        return (
          <div key={category} className="space-y-4">
            <h2 className="text-lg font-semibold text-foreground">{category}</h2>
            <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3">
              {formats.map((format) => (
                <Card key={format.name} className="bg-card border-border">
                  <CardContent className="p-4 space-y-3">
                    <div className="flex items-start justify-between">
                      <div className="w-10 h-10 rounded-lg bg-secondary flex items-center justify-center">
                        <format.icon className="w-5 h-5 text-primary" />
                      </div>
                      <span className="flex items-center gap-1 text-xs text-[hsl(var(--haven-success))]">
                        <CheckCircle className="w-3.5 h-3.5" />
                        Supported
                      </span>
                    </div>
                    <div>
                      <h3 className="text-sm font-semibold text-foreground">{format.name}</h3>
                      <p className="text-xs text-muted-foreground mt-0.5">{format.description}</p>
                      <p className="text-xs font-mono text-muted-foreground mt-1">{format.extensions}</p>
                    </div>
                  </CardContent>
                </Card>
              ))}
            </div>
          </div>
        );
      })}

      <Card className="bg-card border-border">
        <CardHeader>
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-primary/10 border border-primary/20 flex items-center justify-center">
              <Sparkles className="w-5 h-5 text-primary" />
            </div>
            <div>
              <CardTitle className="text-foreground">Document Processing</CardTitle>
              <CardDescription>
                Available extraction tools vary by installation. A supported file extension does not guarantee that all content can be extracted.
              </CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            Document processing uses built-in extractors and, where available, Unstructured.
            Hosted Unstructured processing is available only when configured for your installation.
            Scanned PDFs and complex layouts may need additional processing that is not available in every installation.
          </p>
        </CardContent>
      </Card>
    </div>
  );
};

export default DataTypesPage;
